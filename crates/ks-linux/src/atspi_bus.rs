//! The accessibility bus, spoken to directly with `zbus`' blocking API.
//!
//! libatspi's `atspi_init` finds the address of the accessibility bus (`org.a11y.Bus.GetAddress` on the session bus, or
//! `AT_SPI_BUS_ADDRESS`), connects, and times calls out after `set_timeout`. The same is done here, and nothing else of
//! libatspi is needed: every call is a method of `org.a11y.atspi.*` on a node.

use ks_cast::To as _;
use std::time::Duration;

use zbus::blocking::Connection;
use zbus::blocking::connection::Builder;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

use crate::atspi_context::{AtspiApi, AtspiError, AtspiResult, INTERFACE_TEXT, NodeRef};
use crate::constants::ATSPI_CALL_TIMEOUT_MS;

const REGISTRY_NAME: &str = "org.a11y.atspi.Registry";
const REGISTRY_ROOT: &str = "/org/a11y/atspi/accessible/root";
const INTERFACE_ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
const INTERFACE_EDITABLE: &str = "org.a11y.atspi.EditableText";
const INTERFACE_PROPERTIES: &str = "org.freedesktop.DBus.Properties";
/// The session bus is asked once for the address of the accessibility bus, which may have to be started for it.
const BUS_ADDRESS_TIMEOUT_SECONDS: u64 = 5;

fn call_error(error: impl std::fmt::Display) -> AtspiError {
    AtspiError::Call(error.to_string())
}

fn unavailable(error: impl std::fmt::Display) -> AtspiError {
    AtspiError::Unavailable(error.to_string())
}

/// The accessibility bus, connected.
pub struct ZbusAtspi {
    connection: Connection,
}

/// `atspi_init`: the address of the accessibility bus, and a connection with the per-call timeout of the reader.
pub fn connect() -> AtspiResult<Box<dyn AtspiApi>> {
    let address = match std::env::var("AT_SPI_BUS_ADDRESS") {
        Ok(address) if !address.is_empty() => address,
        _ => {
            let session = Builder::session()
                .map_err(unavailable)?
                .method_timeout(Duration::from_secs(BUS_ADDRESS_TIMEOUT_SECONDS))
                .build()
                .map_err(unavailable)?;
            let message = session
                .call_method(Some("org.a11y.Bus"), "/org/a11y/bus", Some("org.a11y.Bus"), "GetAddress", &())
                .map_err(unavailable)?;
            message.body().deserialize::<String>().map_err(unavailable)?
        }
    };
    let connection = Builder::address(address.as_str())
        .map_err(unavailable)?
        .method_timeout(Duration::from_millis(ATSPI_CALL_TIMEOUT_MS))
        .build()
        .map_err(unavailable)?;
    Ok(Box::new(ZbusAtspi { connection }))
}

impl ZbusAtspi {
    fn target(node: Option<&NodeRef>) -> (&str, &str) {
        match node {
            Some(node) => (node.bus_name.as_str(), node.path.as_str()),
            None => (REGISTRY_NAME, REGISTRY_ROOT),
        }
    }

    fn call<B, R>(&self, node: &NodeRef, interface: &str, method: &str, body: &B) -> AtspiResult<R>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
        R: for<'d> serde::Deserialize<'d> + zbus::zvariant::Type,
    {
        let message = self
            .connection
            .call_method(Some(node.bus_name.as_str()), node.path.as_str(), Some(interface), method, body)
            .map_err(call_error)?;
        message.body().deserialize::<R>().map_err(call_error)
    }

    fn property<R: TryFrom<OwnedValue>>(&self, node: Option<&NodeRef>, interface: &str, name: &str) -> AtspiResult<R> {
        let (destination, path) = Self::target(node);
        let message = self
            .connection
            .call_method(Some(destination), path, Some(INTERFACE_PROPERTIES), "Get", &(interface, name))
            .map_err(call_error)?;
        let value = message.body().deserialize::<OwnedValue>().map_err(call_error)?;
        R::try_from(value).map_err(|_| AtspiError::Call(format!("property {name} has another type")))
    }
}

impl AtspiApi for ZbusAtspi {
    fn child_count(&self, node: Option<&NodeRef>) -> AtspiResult<usize> {
        let count: i32 = self.property(node, INTERFACE_ACCESSIBLE, "ChildCount")?;
        Ok(usize::try_from(count).unwrap_or(0))
    }

    fn child_at(&self, node: Option<&NodeRef>, index: usize) -> AtspiResult<Option<NodeRef>> {
        let (destination, path) = Self::target(node);
        let message = self
            .connection
            .call_method(Some(destination), path, Some(INTERFACE_ACCESSIBLE), "GetChildAtIndex", &index.low::<i32>())
            .map_err(call_error)?;
        let (bus_name, path) = message.body().deserialize::<(String, OwnedObjectPath)>().map_err(call_error)?;
        // A null reference stands for no child.
        Ok((!bus_name.is_empty() && path.as_str() != "/org/a11y/atspi/null").then(|| NodeRef { bus_name, path: path.to_string() }))
    }

    fn name(&self, node: &NodeRef) -> AtspiResult<String> {
        self.property(Some(node), INTERFACE_ACCESSIBLE, "Name")
    }

    /// The process of the application the node belongs to, as the bus daemon knows its connection.
    fn process_id(&self, node: &NodeRef) -> AtspiResult<u32> {
        let message = self
            .connection
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "GetConnectionUnixProcessID",
                &(node.bus_name.as_str(),),
            )
            .map_err(call_error)?;
        message.body().deserialize::<u32>().map_err(call_error)
    }

    fn states(&self, node: &NodeRef) -> AtspiResult<u64> {
        let words: Vec<u32> = self.call(node, INTERFACE_ACCESSIBLE, "GetState", &())?;
        Ok(words.iter().take(2).enumerate().fold(0u64, |bits, (index, word)| bits | (u64::from(*word) << (32 * index))))
    }

    fn role(&self, node: &NodeRef) -> AtspiResult<u32> {
        self.call(node, INTERFACE_ACCESSIBLE, "GetRole", &())
    }

    fn has_interface(&self, node: &NodeRef, interface: &str) -> AtspiResult<bool> {
        let interfaces: Vec<String> = self.call(node, INTERFACE_ACCESSIBLE, "GetInterfaces", &())?;
        Ok(interfaces.iter().any(|name| name == interface))
    }

    fn n_selections(&self, node: &NodeRef) -> AtspiResult<i32> {
        self.call(node, INTERFACE_TEXT, "GetNSelections", &())
    }

    fn caret_offset(&self, node: &NodeRef) -> AtspiResult<i32> {
        self.property(Some(node), INTERFACE_TEXT, "CaretOffset")
    }

    fn character_count(&self, node: &NodeRef) -> AtspiResult<i32> {
        self.property(Some(node), INTERFACE_TEXT, "CharacterCount")
    }

    fn get_text(&self, node: &NodeRef, start: i32, end: i32) -> AtspiResult<String> {
        self.call(node, INTERFACE_TEXT, "GetText", &(start, end))
    }

    fn get_selection(&self, node: &NodeRef, number: i32) -> AtspiResult<(i32, i32)> {
        self.call(node, INTERFACE_TEXT, "GetSelection", &(number,))
    }

    fn set_selection(&self, node: &NodeRef, number: i32, start: i32, end: i32) -> AtspiResult<bool> {
        self.call(node, INTERFACE_TEXT, "SetSelection", &(number, start, end))
    }

    fn delete_text(&self, node: &NodeRef, start: i32, end: i32) -> AtspiResult<bool> {
        self.call(node, INTERFACE_EDITABLE, "DeleteText", &(start, end))
    }

    fn insert_text(&self, node: &NodeRef, position: i32, text: &str, length: i32) -> AtspiResult<bool> {
        self.call(node, INTERFACE_EDITABLE, "InsertText", &(position, text, length))
    }

    fn character_extents(&self, node: &NodeRef, offset: i32) -> AtspiResult<(i32, i32, i32, i32)> {
        // `ATSPI_COORD_TYPE_SCREEN` is 0.
        self.call(node, INTERFACE_TEXT, "GetCharacterExtents", &(offset, 0_u32))
    }
}
