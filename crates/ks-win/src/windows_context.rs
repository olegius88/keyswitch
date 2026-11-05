//! Read a bounded caret range through Windows UI Automation.
//!
//! Called on the engine worker, never on the keyboard hook's thread. IUIAutomation2 timeouts bound provider calls.
//! Password and selected fields are identified before any GetText call. See Microsoft UI Automation TextPattern Overview /
//! IUIAutomation2.
//!
//! The COM calls go through the vtables directly, with only `windows-sys` beneath them. Each slot number below is the
//! position of the method in its interface, the base interface's methods counted; they and the signatures were compared
//! with the `_Vtbl` structures of the `windows` crate 0.62, which are generated from the Windows SDK's metadata.
//!
//! The failures carry what the Python's exceptions carried into the log, and nothing else (`ReaderFailure`): a failed call
//! is `COMError` with its HRESULT, a null interface where comtypes would raise on the first use of it is `ValueError`, and
//! a null string where the Python would call a method on None is `AttributeError`. A field without a text pattern is no
//! failure: Chromium and Qt windows expose a focused element long before they expose its text.

use ks_cast::To as _;
use std::ffi::c_void;
use std::mem::{transmute_copy, zeroed};
use std::ptr::{NonNull, null, null_mut};
use std::thread::{self, ThreadId};
use std::time::Instant;

use ks_backend::constants::FIELD_CONTEXT_MAX_CHARACTERS;
use ks_backend::selected_text::SOURCE_UIA;
use ks_backend::{
    BackendError, FieldContext, FieldRole, ManagedFieldReader, ReaderFailure, RefusalReason, SelectedText, SelectionError,
    SelectionRefused,
};
use ks_text::pystr;
use windows_sys::Win32::Foundation::{SysFreeString, SysStringLen};
use windows_sys::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED, SAFEARRAY};
use windows_sys::Win32::System::Ole::{
    SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetDim, SafeArrayGetLBound, SafeArrayGetUBound, SafeArrayUnaccessData,
};
use windows_sys::Win32::System::Variant::{VARENUM, VARIANT, VT_BOOL, VT_UNKNOWN, VariantClear};
use windows_sys::Win32::UI::Accessibility::CUIAutomation8;
use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
use windows_sys::core::{BSTR, GUID, HRESULT};

use crate::constants::{
    CLSCTX_SERVER, ENDPOINT_END, ENDPOINT_START, FIELD_AFTER_CARET_MAX_CHARACTERS, MILLISECONDS_PER_SECOND,
    SELECTION_COMMAND_MAX_CHARACTERS, UIA_CONNECTION_TIMEOUT_MS, UIA_EDIT_CONTROL_TYPE_ID, UIA_EMBEDDED_OBJECT_CHARACTER, UIA_IS_READ_ONLY_ATTRIBUTE_ID,
    UIA_TEXT_PATTERN_ID, UIA_TRANSACTION_TIMEOUT_MS, UNIT_CHARACTER,
};
use crate::windows_backend::SelectionReader;

const IID_IUIAUTOMATION2: GUID = GUID::from_u128(0x34723aff_0c9d_49d0_9896_7ab52df8cd8a);
const IID_IUIAUTOMATION_TEXT_PATTERN: GUID = GUID::from_u128(0x32eba289_3583_42c9_9c59_3b6d9a1e9b6a);
const CONTEXT_LIMIT: i32 = ks_cast::const_i32(FIELD_CONTEXT_MAX_CHARACTERS);

// Vtable slots: IUnknown
const QUERY_INTERFACE: usize = 0;
const RELEASE: usize = 2;
// IUIAutomation : IUnknown
const AUTOMATION_GET_FOCUSED_ELEMENT: usize = 8;
const AUTOMATION_RESERVED_MIXED_ATTRIBUTE_VALUE: usize = 55;
// IUIAutomation2 : IUIAutomation, which has 58 slots
const AUTOMATION2_PUT_CONNECTION_TIMEOUT: usize = 61;
const AUTOMATION2_PUT_TRANSACTION_TIMEOUT: usize = 63;
// IUIAutomationElement : IUnknown
const ELEMENT_GET_RUNTIME_ID: usize = 4;
const ELEMENT_GET_CURRENT_PATTERN: usize = 16;
const ELEMENT_CURRENT_PROCESS_ID: usize = 20;
const ELEMENT_CURRENT_CONTROL_TYPE: usize = 21;
const ELEMENT_CURRENT_AUTOMATION_ID: usize = 29;
const ELEMENT_CURRENT_IS_PASSWORD: usize = 35;
// IUIAutomationTextPattern : IUnknown
const PATTERN_GET_SELECTION: usize = 5;
const PATTERN_DOCUMENT_RANGE: usize = 7;
// IUIAutomationTextRangeArray : IUnknown
const RANGES_LENGTH: usize = 3;
const RANGES_GET_ELEMENT: usize = 4;
// IUIAutomationTextRange : IUnknown
const RANGE_CLONE: usize = 3;
const RANGE_COMPARE_ENDPOINTS: usize = 5;
const RANGE_GET_ATTRIBUTE_VALUE: usize = 9;
const RANGE_GET_TEXT: usize = 12;
const RANGE_MOVE_ENDPOINT_BY_UNIT: usize = 14;
const RANGE_MOVE_ENDPOINT_BY_RANGE: usize = 15;

type Getter = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT;
type NumberGetter = unsafe extern "system" fn(*mut c_void, *mut i32) -> HRESULT;
type IndexedGetter = unsafe extern "system" fn(*mut c_void, i32, *mut *mut c_void) -> HRESULT;

fn com_error(code: HRESULT) -> ReaderFailure {
    ReaderFailure { failure_type: "provider_error", failure_name: "COMError", failure_code: Some(i64::from(code)), permanent: false }
}

/// comtypes raises ValueError at the first use of an interface a call answered S_OK and null with.
fn value_error() -> ReaderFailure {
    ReaderFailure { failure_type: "provider_error", failure_name: "ValueError", failure_code: None, permanent: false }
}

/// AttributeError: the Python calls a string method on the None a null BSTR becomes.
fn attribute_error() -> ReaderFailure {
    ReaderFailure { failure_type: "provider_error", failure_name: "AttributeError", failure_code: None, permanent: false }
}

/// comtypes' CoInitializeEx and CoCreateInstance raise OSError, which carries no `hresult` for the log.
fn os_error() -> ReaderFailure {
    ReaderFailure { failure_type: "os_error", failure_name: "OSError", failure_code: None, permanent: false }
}

fn check(code: HRESULT) -> Result<(), ReaderFailure> {
    if code < 0 { Err(com_error(code)) } else { Ok(()) }
}

/// The method in slot `index` of the vtable of `object`, as a function of type `F`.
///
/// # Safety
/// `object` is a live COM interface whose vtable has that slot, and `F` is a pointer to a function of that method's
/// signature.
unsafe fn method<F: Copy>(object: *mut c_void, index: usize) -> F {
    const { assert!(size_of::<F>() == size_of::<*const c_void>()) };
    // SAFETY: a COM interface pointer points at a pointer to its vtable, an array of function pointers; the caller vouches
    // for the slot and the signature.
    unsafe {
        let vtable = *object.cast::<*const *const c_void>();
        transmute_copy(&*vtable.add(index))
    }
}

/// A COM interface this reader owns one reference to.
struct ComPtr(NonNull<c_void>);

impl ComPtr {
    fn raw(&self) -> *mut c_void {
        self.0.as_ptr()
    }

    /// `object.Name`, or a method of no arguments that gives an interface.
    fn get(&self, slot: usize) -> Result<Option<ComPtr>, ReaderFailure> {
        let mut out = null_mut();
        // SAFETY: every caller names a slot of this interface with the signature of `Getter`, and `out` is a valid place.
        check(unsafe { method::<Getter>(self.raw(), slot)(self.raw(), &mut out) })?;
        Ok(NonNull::new(out).map(ComPtr))
    }

    /// As `get`, for an interface the next line uses at once.
    fn get_used(&self, slot: usize) -> Result<ComPtr, ReaderFailure> {
        self.get(slot)?.ok_or_else(value_error)
    }

    fn get_indexed(&self, slot: usize, index: i32) -> Result<Option<ComPtr>, ReaderFailure> {
        let mut out = null_mut();
        // SAFETY: every caller names a slot of this interface with the signature of `IndexedGetter`, and `out` is valid.
        check(unsafe { method::<IndexedGetter>(self.raw(), slot)(self.raw(), index, &mut out) })?;
        Ok(NonNull::new(out).map(ComPtr))
    }

    fn get_number(&self, slot: usize) -> Result<i32, ReaderFailure> {
        let mut value = 0;
        // SAFETY: every caller names a slot of this interface with the signature of `NumberGetter`, and `value` is valid.
        check(unsafe { method::<NumberGetter>(self.raw(), slot)(self.raw(), &mut value) })?;
        Ok(value)
    }

    /// `supported.QueryInterface(interface)`.
    fn query_interface(&self, interface: &GUID) -> Result<ComPtr, ReaderFailure> {
        type QueryInterface = unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT;
        let mut out = null_mut();
        // SAFETY: every COM interface starts with IUnknown, whose first slot has this signature; `out` is valid.
        check(unsafe { method::<QueryInterface>(self.raw(), QUERY_INTERFACE)(self.raw(), interface, &mut out) })?;
        NonNull::new(out).map(ComPtr).ok_or_else(value_error)
    }
}

impl Drop for ComPtr {
    fn drop(&mut self) {
        type Release = unsafe extern "system" fn(*mut c_void) -> u32;
        // SAFETY: this owns one reference, given back once; every COM interface starts with IUnknown.
        unsafe { method::<Release>(self.raw(), RELEASE)(self.raw()) };
    }
}

/// Takes the string a call handed over, frees it, and gives its text; None for a null BSTR.
fn take_bstr(value: BSTR) -> Option<String> {
    if value.is_null() {
        return None;
    }
    // SAFETY: `value` is a live BSTR this code owns, SysStringLen gives its length in units, and it is freed once, after
    // the copy.
    unsafe {
        let units = std::slice::from_raw_parts(value, SysStringLen(value).to::<usize>());
        let text = String::from_utf16_lossy(units);
        SysFreeString(value);
        Some(text)
    }
}

fn get_string(object: &ComPtr, slot: usize) -> Result<Option<String>, ReaderFailure> {
    type StringGetter = unsafe extern "system" fn(*mut c_void, *mut BSTR) -> HRESULT;
    let mut value: BSTR = null();
    // SAFETY: every caller names a slot of this interface with this signature, and `value` is a valid place.
    check(unsafe { method::<StringGetter>(object.raw(), slot)(object.raw(), &mut value) })?;
    Ok(take_bstr(value))
}

fn focused_element(automation: &ComPtr) -> Result<ComPtr, ReaderFailure> {
    automation.get_used(AUTOMATION_GET_FOCUSED_ELEMENT)
}

fn element_process_id(element: &ComPtr) -> Result<i32, ReaderFailure> {
    element.get_number(ELEMENT_CURRENT_PROCESS_ID)
}

fn element_is_password(element: &ComPtr) -> Result<bool, ReaderFailure> {
    Ok(element.get_number(ELEMENT_CURRENT_IS_PASSWORD)? != 0)
}

/// `tuple(element.GetRuntimeId())`: a null array and one of no dimensions are empty, as comtypes unpacks them.
fn runtime_id(element: &ComPtr) -> Result<Vec<i32>, ReaderFailure> {
    type GetRuntimeId = unsafe extern "system" fn(*mut c_void, *mut *mut SAFEARRAY) -> HRESULT;
    let mut array = null_mut();
    // SAFETY: the slot has this signature, and `array` is a valid place.
    check(unsafe { method::<GetRuntimeId>(element.raw(), ELEMENT_GET_RUNTIME_ID)(element.raw(), &mut array) })?;
    if array.is_null() {
        return Ok(Vec::new());
    }
    // SAFETY: `array` is a live SAFEARRAY of VT_I4 this code owns and destroys once, after the copy. The data pointer is
    // used only between AccessData and UnaccessData, and within the bounds the array states.
    unsafe {
        let mut identity = Vec::new();
        if SafeArrayGetDim(array) == 1 {
            let (mut lower, mut upper) = (0, 0);
            let mut data = null_mut();
            if SafeArrayGetLBound(array, 1, &mut lower) >= 0
                && SafeArrayGetUBound(array, 1, &mut upper) >= 0
                && SafeArrayAccessData(array, &mut data) >= 0
            {
                let count = usize::try_from(i64::from(upper) + 1 - i64::from(lower)).unwrap_or(0);
                // An empty array may have no data at all.
                if count > 0 && !data.is_null() {
                    identity.extend_from_slice(std::slice::from_raw_parts(data.cast::<i32>(), count));
                }
                SafeArrayUnaccessData(array);
            }
        }
        SafeArrayDestroy(array);
        Ok(identity)
    }
}

fn selection_ranges(pattern: &ComPtr) -> Result<ComPtr, ReaderFailure> {
    pattern.get_used(PATTERN_GET_SELECTION)
}

fn compare_endpoints(range: &ComPtr, endpoint: i32, other: &ComPtr, other_endpoint: i32) -> Result<i32, ReaderFailure> {
    type CompareEndpoints = unsafe extern "system" fn(*mut c_void, i32, *mut c_void, i32, *mut i32) -> HRESULT;
    let mut order = 0;
    // SAFETY: the slot has this signature, `other` is a live range, and `order` is a valid place.
    check(unsafe {
        method::<CompareEndpoints>(range.raw(), RANGE_COMPARE_ENDPOINTS)(range.raw(), endpoint, other.raw(), other_endpoint, &mut order)
    })?;
    Ok(order)
}

fn move_endpoint_by_unit(range: &ComPtr, endpoint: i32, count: i32) -> Result<(), ReaderFailure> {
    type MoveEndpointByUnit = unsafe extern "system" fn(*mut c_void, i32, i32, i32, *mut i32) -> HRESULT;
    let mut moved = 0;
    // SAFETY: the slot has this signature, and `moved` is a valid place.
    check(unsafe {
        method::<MoveEndpointByUnit>(range.raw(), RANGE_MOVE_ENDPOINT_BY_UNIT)(range.raw(), endpoint, UNIT_CHARACTER, count, &mut moved)
    })
}

fn move_endpoint_by_range(range: &ComPtr, endpoint: i32, other: &ComPtr, other_endpoint: i32) -> Result<(), ReaderFailure> {
    type MoveEndpointByRange = unsafe extern "system" fn(*mut c_void, i32, *mut c_void, i32) -> HRESULT;
    // SAFETY: the slot has this signature, and `other` is a live range.
    check(unsafe {
        method::<MoveEndpointByRange>(range.raw(), RANGE_MOVE_ENDPOINT_BY_RANGE)(range.raw(), endpoint, other.raw(), other_endpoint)
    })
}

fn get_text(range: &ComPtr, maximum: i32) -> Result<Option<String>, ReaderFailure> {
    type GetText = unsafe extern "system" fn(*mut c_void, i32, *mut BSTR) -> HRESULT;
    let mut value: BSTR = null();
    // SAFETY: the slot has this signature, and `value` is a valid place.
    check(unsafe { method::<GetText>(range.raw(), RANGE_GET_TEXT)(range.raw(), maximum, &mut value) })?;
    Ok(take_bstr(value))
}

/// The text of a range, with the placeholder of an embedded object removed: `GetText(n).replace(...)`.
fn text_without_placeholder(range: &ComPtr, maximum: i32) -> Result<String, ReaderFailure> {
    let text = get_text(range, maximum)?.ok_or_else(attribute_error)?;
    Ok(text.replace(UIA_EMBEDDED_OBJECT_CHARACTER, ""))
}

/// Whether the selection says it is read-only: VT_BOOL comes back as a bool. A selection partly read-only answers with the
/// mixed-attribute token (IUIAutomation::ReservedMixedAttributeValue), and a provider without the attribute with another
/// token. comtypes compares the tokens by their pointers.
fn selection_is_read_only(automation: &ComPtr, selection: &ComPtr) -> Result<bool, ReaderFailure> {
    type GetAttributeValue = unsafe extern "system" fn(*mut c_void, i32, *mut VARIANT) -> HRESULT;
    // SAFETY: VARIANT is plain data and zero is VT_EMPTY.
    let mut value: VARIANT = unsafe { zeroed() };
    // SAFETY: the slot has this signature, and `value` is a valid, empty VARIANT.
    check(unsafe {
        method::<GetAttributeValue>(selection.raw(), RANGE_GET_ATTRIBUTE_VALUE)(selection.raw(), UIA_IS_READ_ONLY_ATTRIBUTE_ID, &mut value)
    })?;
    // SAFETY: the call filled the VARIANT: `vt` says which member of the union holds the value, and only that one is read.
    let (kind, flag, token): (VARENUM, bool, *mut c_void) = unsafe {
        let inner = &value.Anonymous.Anonymous;
        match inner.vt {
            VT_BOOL => (VT_BOOL, inner.Anonymous.boolVal != 0, null_mut()),
            VT_UNKNOWN => (VT_UNKNOWN, false, inner.Anonymous.punkVal),
            other => (other, false, null_mut()),
        }
    };
    let answer = if kind == VT_BOOL && flag {
        Ok(true)
    } else {
        // The token is read only here, as the Python's `or` asks for it. The VARIANT keeps `token` alive meanwhile.
        automation
            .get(AUTOMATION_RESERVED_MIXED_ATTRIBUTE_VALUE)
            .map(|mixed| !token.is_null() && mixed.is_some_and(|mixed| mixed.raw() == token))
    };
    // SAFETY: `value` was filled by the call above and is cleared once.
    unsafe { VariantClear(&mut value) };
    answer
}

fn process_for_window(window: i64) -> i64 {
    let mut pid = 0u32;
    // SAFETY: `pid` is a valid place; a stale window leaves it 0.
    unsafe { GetWindowThreadProcessId(window.to::<isize>().pointer_mut::<c_void>(), &mut pid) };
    i64::from(pid)
}

fn uia_field(application: &str, field_id: String) -> FieldContext {
    let mut field = FieldContext::new(application, field_id);
    field.source = SOURCE_UIA.to_owned();
    field
}

fn password_field(application: &str, field_id: String) -> FieldContext {
    let mut field = uia_field(application, field_id);
    field.role = FieldRole::Password;
    field.sensitive = true;
    field
}

pub struct WindowsFieldReader {
    automation: Option<ComPtr>,
    thread: ThreadId,
    /// This thread entered the apartment and owes it one CoUninitialize.
    com: bool,
}

// SAFETY: the object comes from the multithreaded apartment (COINIT_MULTITHREADED), whose interfaces may be called from
// any of its threads, and the backend keeps the reader behind a mutex. In practice only the engine's worker ever uses it,
// as in the Python.
unsafe impl Send for WindowsFieldReader {}

impl WindowsFieldReader {
    /// The worker thread has its own COM apartment. No UIA objects are passed between the hook/UI and worker threads.
    pub fn new() -> Result<Self, ReaderFailure> {
        // SAFETY: the reserved pointer must be null. S_FALSE also counts as an entry to the apartment that needs its
        // CoUninitialize; a thread already in another apartment gets RPC_E_CHANGED_MODE.
        if unsafe { CoInitializeEx(null(), COINIT_MULTITHREADED.to::<u32>()) } < 0 {
            return Err(os_error());
        }
        let mut reader = WindowsFieldReader { automation: None, thread: thread::current().id(), com: true };
        let mut out = null_mut();
        // SAFETY: both GUIDs are valid for the call, no outer object is passed, and `out` is a valid place.
        let created =
            unsafe { CoCreateInstance(&CUIAutomation8, null_mut(), CLSCTX_SERVER, &IID_IUIAUTOMATION2, &mut out) };
        if created < 0 {
            return Err(os_error());
        }
        let automation = NonNull::new(out).map(ComPtr).ok_or_else(value_error)?;
        // Both are milliseconds. UI Automation's own defaults are seconds (IUIAutomation2::put_TransactionTimeout: "The
        // default transaction timeout value is 20 seconds"), which would stall the engine worker, so KeySwitch bounds them
        // hard. The earlier bound of 50 ms was tighter than Chromium and Qt windows can answer on a first, cold request, and
        // every such timeout counted as a broken provider; the budget below still keeps a word's decision inside a fraction
        // of a second, and `last_read_ms` reports what it costs.
        for (slot, timeout) in [
            (AUTOMATION2_PUT_CONNECTION_TIMEOUT, UIA_CONNECTION_TIMEOUT_MS),
            (AUTOMATION2_PUT_TRANSACTION_TIMEOUT, UIA_TRANSACTION_TIMEOUT_MS),
        ] {
            type PutTimeout = unsafe extern "system" fn(*mut c_void, u32) -> HRESULT;
            // SAFETY: the object was asked for as IUIAutomation2, whose slots these are, with this signature.
            check(unsafe { method::<PutTimeout>(automation.raw(), slot)(automation.raw(), timeout) })?;
        }
        reader.automation = Some(automation);
        Ok(reader)
    }

    fn automation(&self) -> Result<&ComPtr, ReaderFailure> {
        // A closed reader has no `automation` attribute in the Python either.
        self.automation.as_ref().ok_or_else(attribute_error)
    }

    /// The field around the caret. None for a field that cannot be read at all.
    pub fn read(&self, application: &str, window: i64) -> Result<Option<FieldContext>, ReaderFailure> {
        let automation = self.automation()?;
        let element = focused_element(automation)?;
        if i64::from(element_process_id(&element)?) != process_for_window(window) {
            return Ok(None);
        }
        let identity = runtime_id(&element)?;
        let field_id = identity.iter().map(i32::to_string).collect::<Vec<_>>().join(":");
        if element_is_password(&element)? {
            return Ok(Some(password_field(application, field_id)));
        }
        let Some(supported) = element.get_indexed(ELEMENT_GET_CURRENT_PATTERN, UIA_TEXT_PATTERN_ID)? else {
            // Chromium and Qt windows expose a focused element long before they expose its text. That is a field KeySwitch
            // cannot read, not a provider that has to be rebuilt: GetCurrentPattern answers S_OK with a null pointer.
            return Ok(None);
        };
        let pattern = supported.query_interface(&IID_IUIAUTOMATION_TEXT_PATTERN)?;
        let ranges = selection_ranges(&pattern)?;
        if ranges.get_number(RANGES_LENGTH)? > 1 {
            let mut field = uia_field(application, field_id);
            field.selection = true;
            return Ok(Some(field));
        }
        if ranges.get_number(RANGES_LENGTH)? != 1 {
            return Ok(None);
        }
        let caret = ranges.get_indexed(RANGES_GET_ELEMENT, 0)?.ok_or_else(value_error)?;
        if compare_endpoints(&caret, ENDPOINT_START, &caret, ENDPOINT_END)? != 0 {
            let mut field = uia_field(application, field_id);
            field.selection = true;
            return Ok(Some(field));
        }
        let before = caret.get_used(RANGE_CLONE)?;
        let after = caret.get_used(RANGE_CLONE)?;
        move_endpoint_by_unit(&before, ENDPOINT_START, -CONTEXT_LIMIT)?;
        move_endpoint_by_unit(&after, ENDPOINT_END, FIELD_AFTER_CARET_MAX_CHARACTERS)?;
        // A browser's text ranges run through the whole page: Chromium moves an endpoint past the edge of the focused input
        // into the text around it. An empty input in Edge read 512 characters of the paragraph above it and the fields below,
        // and an empty chat box in VS Code read the conversation above it: every read in VS Code held 450 or more characters
        // before the caret and some after it, and a Russian `ты` typed there became `ns`. Firefox reads there looked the same.
        // The field's own document range is its text alone, so both reads stop at its edges.
        let field = pattern.get_used(PATTERN_DOCUMENT_RANGE)?;
        if compare_endpoints(&before, ENDPOINT_START, &field, ENDPOINT_START)? < 0 {
            move_endpoint_by_range(&before, ENDPOINT_START, &field, ENDPOINT_START)?;
        }
        if compare_endpoints(&after, ENDPOINT_END, &field, ENDPOINT_END)? > 0 {
            move_endpoint_by_range(&after, ENDPOINT_END, &field, ENDPOINT_END)?;
        }
        let prefix = text_without_placeholder(&before, CONTEXT_LIMIT)?;
        let suffix = text_without_placeholder(&after, FIELD_AFTER_CARET_MAX_CHARACTERS)?;
        let current = focused_element(automation)?;
        if element_is_password(&current)? {
            return Ok(Some(password_field(application, field_id)));
        }
        if runtime_id(&current)? != identity {
            // Explicit contradictory evidence, not an unsupported provider. An empty native suffix makes the policy reject
            // stale strokes.
            return Ok(Some(uia_field(application, field_id)));
        }
        let automation_id = get_string(&element, ELEMENT_CURRENT_AUTOMATION_ID)?.ok_or_else(attribute_error)?;
        let mut result = uia_field(application, field_id);
        result.before = prefix;
        result.after = suffix;
        result.role = if pystr::casefold(&automation_id).contains("search") { FieldRole::Search } else { FieldRole::Text };
        Ok(Some(result.bounded()))
    }

    /// The text selected in the focused field of `window`.
    ///
    /// None where UI Automation cannot say (no text pattern, another process in focus): the clipboard is asked then.
    /// SelectionRefused where it can and the selection is not one to type over: a password field, no selection or several,
    /// text that says it is read-only, or an embedded object (a picture in a rich editor) that typing would delete. One
    /// character past the hotkeys' bound is read, so that a longer selection is told apart.
    pub fn read_selection(&self, window: i64) -> Result<Option<SelectedText>, SelectionFailure> {
        let automation = self.automation()?;
        let element = focused_element(automation)?;
        if i64::from(element_process_id(&element)?) != process_for_window(window) {
            return Ok(None);
        }
        if element_is_password(&element)? {
            return Err(RefusalReason::SensitiveField.into());
        }
        let Some(supported) = element.get_indexed(ELEMENT_GET_CURRENT_PATTERN, UIA_TEXT_PATTERN_ID)? else {
            return Ok(None);
        };
        let pattern = supported.query_interface(&IID_IUIAUTOMATION_TEXT_PATTERN)?;
        let ranges = selection_ranges(&pattern)?;
        if ranges.get_number(RANGES_LENGTH)? > 1 {
            return Err(RefusalReason::MultipleSelection.into());
        }
        if ranges.get_number(RANGES_LENGTH)? != 1 {
            return Err(RefusalReason::NoSelection.into());
        }
        let selection = ranges.get_indexed(RANGES_GET_ELEMENT, 0)?.ok_or_else(value_error)?;
        if compare_endpoints(&selection, ENDPOINT_START, &selection, ENDPOINT_END)? == 0 {
            return Err(RefusalReason::NoSelection.into());
        }
        if selection_is_read_only(automation, &selection)? {
            return Err(RefusalReason::ReadOnly.into());
        }
        let text = get_text(&selection, (SELECTION_COMMAND_MAX_CHARACTERS + 1).low::<i32>())?.ok_or_else(attribute_error)?;
        if text.contains(UIA_EMBEDDED_OBJECT_CHARACTER) {
            return Err(RefusalReason::Untypeable.into());
        }
        Ok(Some(SelectedText { text, source: SOURCE_UIA.to_owned() }))
    }

    /// Whether the focused element of `window` is an edit field, which may still have no text pattern.
    pub fn focused_editable(&self, window: i64) -> Result<bool, ReaderFailure> {
        let element = focused_element(self.automation()?)?;
        Ok(i64::from(element_process_id(&element)?) == process_for_window(window)
            && element.get_number(ELEMENT_CURRENT_CONTROL_TYPE)? == UIA_EDIT_CONTROL_TYPE_ID)
    }

    pub fn close(&mut self) {
        if self.com && thread::current().id() == self.thread {
            // Drop all apartment-bound interface references before uninit.
            self.automation = None;
            // SAFETY: this thread entered the apartment in `new`, and `com` makes this the one matching call.
            unsafe { CoUninitialize() };
            self.com = false;
        }
    }
}

impl Drop for WindowsFieldReader {
    fn drop(&mut self) {
        self.close();
    }
}

/// `read_selection` has two ways of not answering, as the Python raises two kinds of exception.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionFailure {
    Refused(RefusalReason),
    Failed(ReaderFailure),
}

impl From<RefusalReason> for SelectionFailure {
    fn from(reason: RefusalReason) -> Self {
        SelectionFailure::Refused(reason)
    }
}

impl From<ReaderFailure> for SelectionFailure {
    fn from(failure: ReaderFailure) -> Self {
        SelectionFailure::Failed(failure)
    }
}

impl From<SelectionFailure> for SelectionError {
    fn from(failure: SelectionFailure) -> Self {
        match failure {
            SelectionFailure::Refused(reason) => SelectionRefused::new(reason).into(),
            SelectionFailure::Failed(failure) => BackendError::Failed(failure.failure_name.to_owned()).into(),
        }
    }
}

impl ManagedFieldReader for WindowsFieldReader {
    fn read(&self, application: &str, window: i64) -> Result<Option<FieldContext>, ReaderFailure> {
        WindowsFieldReader::read(self, application, window)
    }

    fn close(&mut self) {
        WindowsFieldReader::close(self);
    }
}

impl SelectionReader for WindowsFieldReader {
    fn read_selection(&self, window: i64) -> Result<Option<SelectedText>, SelectionError> {
        WindowsFieldReader::read_selection(self, window).map_err(SelectionError::from)
    }

    fn focused_editable(&self, window: i64) -> Result<bool, SelectionError> {
        WindowsFieldReader::focused_editable(self, window)
            .map_err(|failure| BackendError::Failed(failure.failure_name.to_owned()).into())
    }
}

/// What the explicit diagnostic readiness probe found; each None is a key the Python's dictionary leaves out, but for
/// `focused_text_pattern`, which it writes as null after a failure.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UiaProbe {
    pub available: bool,
    pub error: Option<&'static str>,
    pub focused_text_pattern: Option<bool>,
    pub focused_probe_ms: Option<i64>,
    pub focused_error: Option<&'static str>,
    pub hresult: Option<i64>,
}

/// Explicit diagnostic readiness probe; no field text is read.
///
/// Besides building the bridge it asks the focused element whether it offers TextPattern at all and how long that answer
/// took. Both are facts about the provider, not about what the user typed: no range is ever read here.
pub fn probe_uia() -> UiaProbe {
    let mut reader = match WindowsFieldReader::new() {
        Ok(reader) => reader,
        Err(failure) => {
            return UiaProbe {
                available: false,
                error: Some(failure.failure_name),
                hresult: failure.failure_code,
                ..UiaProbe::default()
            };
        }
    };
    let started = Instant::now();
    let answer = reader.automation().and_then(focused_element).and_then(|element| {
        element.get_indexed(ELEMENT_GET_CURRENT_PATTERN, UIA_TEXT_PATTERN_ID)
    });
    let probe = match answer {
        Ok(pattern) => UiaProbe {
            available: true,
            focused_text_pattern: Some(pattern.is_some()),
            focused_probe_ms: Some(((started.elapsed().as_secs_f64() * MILLISECONDS_PER_SECOND).round_ties_even()).to::<i64>()),
            ..UiaProbe::default()
        },
        Err(failure) => UiaProbe {
            available: true,
            focused_error: Some(failure.failure_name),
            hresult: failure.failure_code,
            ..UiaProbe::default()
        },
    };
    reader.close();
    probe
}
