from types import TracebackType

HKEY_CURRENT_USER: int
HKEY_LOCAL_MACHINE: int
KEY_READ: int
KEY_SET_VALUE: int
KEY_WOW64_32KEY: int
KEY_WOW64_64KEY: int
REG_SZ: int

class HKEYType:
    def __enter__(self) -> HKEYType: ...
    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc_value: BaseException | None,
        traceback: TracebackType | None,
    ) -> bool | None: ...

def OpenKey(
    key: int | HKEYType,
    sub_key: str,
    reserved: int = ...,
    access: int = ...,
) -> HKEYType: ...
def CreateKeyEx(
    key: int | HKEYType,
    sub_key: str,
    reserved: int = ...,
    access: int = ...,
) -> HKEYType: ...
def QueryValueEx(key: HKEYType, value_name: str) -> tuple[object, int]: ...
def SetValueEx(
    key: HKEYType,
    value_name: str,
    reserved: int,
    value_type: int,
    value: str,
) -> None: ...
def DeleteValue(key: HKEYType, value_name: str) -> None: ...
def EnumKey(key: HKEYType, index: int) -> str: ...
