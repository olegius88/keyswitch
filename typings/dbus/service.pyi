from collections.abc import Callable
from typing import ParamSpec, TypeVar

_P = ParamSpec("_P")
_R = TypeVar("_R")

class BusName:
    def __init__(
        self,
        name: str,
        bus: object = ...,
        allow_replacement: bool = ...,
        replace_existing: bool = ...,
        do_not_queue: bool = ...,
    ) -> None: ...
    def get_name(self) -> str: ...

class Object:
    def __init__(self, bus_name: BusName | object, object_path: str) -> None: ...
    def remove_from_connection(self) -> None: ...

class Interface:
    def RegisterStatusNotifierItem(self, service: str) -> None: ...

def method(
    dbus_interface: str,
    in_signature: str = ...,
    out_signature: str = ...,
    **kwargs: object,
) -> Callable[[Callable[_P, _R]], Callable[_P, _R]]: ...

def signal(
    dbus_interface: str,
    signature: str = ...,
    **kwargs: object,
) -> Callable[[Callable[_P, _R]], Callable[_P, _R]]: ...
