from collections.abc import Iterable, Mapping
from typing import Generic, TypeVar

from . import exceptions as exceptions
from . import service as service

_T = TypeVar("_T")
_K = TypeVar("_K")
_V = TypeVar("_V")

class DBusException(Exception): ...

class Boolean(int):
    def __new__(
        cls, value: object = ..., variant_level: int = ...
    ) -> Boolean: ...

class Byte(int):
    def __new__(cls, value: int = ..., variant_level: int = ...) -> Byte: ...

class Int32(int):
    def __new__(cls, value: int = ..., variant_level: int = ...) -> Int32: ...

class UInt32(int):
    def __new__(cls, value: int = ..., variant_level: int = ...) -> UInt32: ...

class String(str):
    def __new__(
        cls, value: object = ..., variant_level: int = ...
    ) -> String: ...

class ObjectPath(str):
    def __new__(
        cls, value: object = ..., variant_level: int = ...
    ) -> ObjectPath: ...

class Array(list[_T], Generic[_T]):
    def __init__(
        self,
        iterable: Iterable[_T] = ...,
        signature: str | None = ...,
        variant_level: int = ...,
    ) -> None: ...

class Dictionary(dict[_K, _V], Generic[_K, _V]):
    def __init__(
        self,
        mapping: Mapping[_K, _V] | Iterable[tuple[_K, _V]] = ...,
        signature: str | None = ...,
        variant_level: int = ...,
    ) -> None: ...

class Struct(tuple[object, ...]):
    def __new__(
        cls,
        values: Iterable[object] = ...,
        signature: str | None = ...,
        variant_level: int = ...,
    ) -> Struct: ...

class ProxyObject: ...

class Bus:
    def get_object(self, bus_name: str, object_path: str) -> ProxyObject: ...

class SessionBus(Bus): ...

def Interface(
    proxy_object: object, dbus_interface: str | None = ...
) -> service.Interface: ...
