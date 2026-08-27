from collections.abc import Callable

class Icon:
    icon: object
    title: str
    def __init__(
        self,
        name: str,
        icon: object = ...,
        title: str = ...,
        menu: Menu | None = ...,
    ) -> None: ...
    def run_detached(self) -> None: ...
    def update_menu(self) -> None: ...
    def notify(self, message: str, title: str | None = ...) -> None: ...
    def stop(self) -> None: ...
    def _on_notify(self, wparam: int, lparam: int) -> None: ...

class MenuItem:
    def __init__(
        self,
        text: str | Callable[[MenuItem], str],
        action: Callable[[Icon, MenuItem], object] | None,
        *,
        checked: Callable[[MenuItem], bool] | None = ...,
        enabled: bool | Callable[[MenuItem], bool] = ...,
        default: bool = ...,
    ) -> None: ...

class Menu:
    SEPARATOR: MenuItem
    def __init__(self, *items: MenuItem) -> None: ...
