"""Declarative settings catalogue shared by the Windows UI and tests."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal


ControlKind = Literal["bool", "choice", "int", "float", "text"]


@dataclass(frozen=True)
class SettingSpec:
    path: str
    title: str
    description: str
    kind: ControlKind
    choices: tuple[tuple[str, str], ...] = ()
    minimum: float = 0.0
    maximum: float = 100.0
    step: float = 1.0


AUTOCORRECTION_SETTINGS = (
    SettingSpec(
        "enabled",
        "Автоматически исправлять раскладку",
        "Анализировать завершённые слова и исправлять уверенные ошибки.",
        "bool",
    ),
    SettingSpec(
        "detection.minimum_length",
        "Минимальная длина слова",
        "Минимум для базового детектора; явные правила, короткие исключения и контекстный помощник могут разрешить замену раньше.",
        "int",
        minimum=2,
        maximum=12,
    ),
    SettingSpec(
        "detection.confidence",
        "Порог уверенности",
        "Больше — строже резервные эвристики. Пороги обученных моделей не меняются.",
        "float",
        minimum=0.5,
        maximum=10.0,
        step=0.1,
    ),
    SettingSpec(
        "detection.respect_manual_layout",
        "Учитывать ручную смену языка",
        "Не исправлять первое слово после выбранной пользователем раскладки.",
        "bool",
    ),
    SettingSpec(
        "detection.context_aware",
        "Учитывать предыдущие слова",
        "Использовать недавний языковой контекст текущего приложения.",
        "bool",
    ),
    SettingSpec(
        "detection.context_policy",
        "Контекстный ИИ-помощник",
        "Локальная модель учитывает фразу и приложение. Наблюдение записывает только решение и размеры контекста, не текст фразы.",
        "choice",
        choices=(("assist", "Исправлять и предлагать"), ("shadow", "Только наблюдать"), ("off", "Выключен")),
    ),
    SettingSpec(
        "detection.context_read_field",
        "Читать контекст активного поля",
        "Читать текст рядом с курсором локально через доступность ОС. Распознанные защищённые поля исключаются; не все приложения сообщают о них.",
        "bool",
    ),
    SettingSpec(
        "detection.protect_code",
        "Защищать код, URL и сокращения",
        "Оставлять технические токены и адреса без автокоррекции.",
        "bool",
    ),
    SettingSpec(
        "detection.intent_model_enabled",
        "Локальная линейная модель",
        "Базовый KSLM-распознаватель слов; при отключении работают резервные эвристики. Контекстный помощник настраивается отдельно.",
        "bool",
    ),
    SettingSpec(
        "detection.aggressive",
        "Расширенное распознавание",
        "Расширить резервное распознавание незнакомых слов; пороги обученных моделей не меняются.",
        "bool",
    ),
    SettingSpec(
        "detection.early_switch",
        "Ранняя смена раскладки",
        "Переключение по префиксу в off/shadow. При работающем помощнике assist проверка выполняется после слова или паузы в наборе.",
        "bool",
    ),
    SettingSpec(
        "detection.early_switch_min_length",
        "Букв до ранней смены",
        "Сколько букв нужно набрать, прежде чем префикс может переключить раскладку. Меньше — быстрее, но чаще ложные переключения на сокращениях.",
        "int",
        minimum=3,
        maximum=8,
    ),
    SettingSpec(
        "detection.learning",
        "Локальное обучение",
        "Предлагать правило после Pause/Break и запоминать подтверждение Enter.",
        "bool",
    ),
    SettingSpec(
        "detection.learning_confirmations",
        "Подтверждений для правила",
        "Сколько повторов нужно, если предложение не подтверждено клавишей Enter.",
        "int",
        minimum=1,
        maximum=10,
    ),
)


TRIGGER_SETTINGS = (
    SettingSpec(
        "detection.correct_on_pause",
        "Пауза в наборе",
        "Проверять текущее слово после паузы без ввода.",
        "bool",
    ),
    SettingSpec(
        "detection.pause_delay_seconds",
        "Пауза, секунд",
        "Сколько ждать без ввода, прежде чем проверить незавершённое слово.",
        "float",
        minimum=0.3,
        maximum=5.0,
        step=0.1,
    ),
    SettingSpec(
        "detection.correct_on_space",
        "Пробел",
        "Проверять слово перед пробелом.",
        "bool",
    ),
    SettingSpec(
        "detection.correct_on_enter",
        "Перед Enter",
        "Сначала исправить слово, затем передать Enter приложению один раз. Shift+Enter не перехватывается.",
        "bool",
    ),
    SettingSpec(
        "detection.correct_on_tab",
        "Перед Tab",
        "Сначала исправить слово, затем перейти к следующему полю. Shift+Tab не перехватывается.",
        "bool",
    ),
    SettingSpec(
        "detection.correct_on_punctuation",
        "Знаки препинания",
        "Проверять слово перед точкой, запятой и другими разделителями.",
        "bool",
    ),
)


HOTKEY_SETTINGS = (
    SettingSpec(
        "hotkeys.toggle",
        "Включить или приостановить",
        "Глобальная пауза автоматического переключения.",
        "text",
    ),
    SettingSpec(
        "hotkeys.convert_last",
        "Преобразовать последнее слово",
        "Ручная смена раскладки уже введённого слова.",
        "text",
    ),
    SettingSpec(
        "hotkeys.undo",
        "Отменить исправление",
        "Вернуть последнее автоматическое исправление в течение 10 секунд.",
        "text",
    ),
)


SYSTEM_SETTINGS = (
    SettingSpec(
        "general.autostart",
        "Запускать вместе с Windows",
        "Стартовать после входа пользователя в систему.",
        "bool",
    ),
    SettingSpec(
        "general.start_hidden",
        "Запускать свёрнутым в трей",
        "Не показывать окно настроек после автоматического запуска.",
        "bool",
    ),
    SettingSpec(
        "general.close_to_tray",
        "Закрывать окно в трей",
        "Кнопка закрытия скрывает окно, а движок продолжает работать.",
        "bool",
    ),
    SettingSpec(
        "appearance.show_indicator",
        "Показывать индикатор в области уведомлений",
        "Иконка показывает текущую раскладку и открывает меню KeySwitch.",
        "bool",
    ),
    SettingSpec(
        "appearance.indicator_style",
        "Вид индикатора",
        "Двухбуквенное обозначение или флаг страны.",
        "choice",
        choices=(("letters", "EN / RU"), ("flags", "Флаги стран")),
    ),
    SettingSpec(
        "appearance.theme",
        "Тема окна",
        "Системная, светлая или тёмная цветовая схема.",
        "choice",
        choices=(("system", "Системная"), ("light", "Светлая"), ("dark", "Тёмная")),
    ),
    SettingSpec(
        "general.notifications",
        "Уведомления об исправлениях",
        "Показывать исходное и исправленное слово.",
        "bool",
    ),
    SettingSpec(
        "general.sound",
        "Звук исправления",
        "Воспроизводить системный сигнал после замены.",
        "bool",
    ),
    SettingSpec(
        "general.keep_history",
        "Сохранять историю",
        "Записывать локально только пары исправленных слов.",
        "bool",
    ),
    SettingSpec(
        "history.limit",
        "Размер истории",
        "Максимальное число локально сохранённых исправлений.",
        "int",
        minimum=10,
        maximum=5000,
        step=10,
    ),
)


UPDATE_SETTINGS = (
    SettingSpec(
        "updates.check_automatically",
        "Автоматически проверять обновления",
        "Проверять GitHub Releases после запуска и каждые шесть часов.",
        "bool",
    ),
    SettingSpec(
        "updates.install_automatically",
        "Автоматически устанавливать обновления",
        "Скачать проверенный Setup EXE, тихо обновить KeySwitch и перезапустить его.",
        "bool",
    ),
)


DIAGNOSTIC_SETTINGS = (
    SettingSpec(
        "diagnostics.technical_logging",
        "Записывать технический журнал",
        "Сохранять решения распознавания, оценки и названия приложений. Журнал может содержать введённые слова; в приложениях-исключениях текст скрывается.",
        "bool",
    ),
)


ALL_SETTING_SPECS = (
    *AUTOCORRECTION_SETTINGS,
    *TRIGGER_SETTINGS,
    *HOTKEY_SETTINGS,
    *SYSTEM_SETTINGS,
    *UPDATE_SETTINGS,
    *DIAGNOSTIC_SETTINGS,
)
