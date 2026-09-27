# Application
app-name = WMDE Viewer
app-description = Переглядач зображень для стільниці WMDE

## About
menu-about = Про WMDE Viewer...
repository = Сховище
support = Підтримка

# Menu - File
menu-file = Файл
menu-open-file = Відкрити файл
menu-open-recent = Нещодавні
menu-open-folder = Відкрити теку
menu-open-containing = Відкрити розташування файлу
menu-save = Зберегти
menu-save-as = Зберегти як
menu-set-wallpaper = Зробити тлом
menu-move-to-trash = Пересунути до смітника
menu-delete-permanently = Остаточно видалити
menu-image-details = Відомості про зображення
menu-quit = Завершити

# Wallpaper dialog
wallpaper-dialog-title = Зробити тлом
wallpaper-all-displays = Всі дисплеї
wallpaper-cancel = Скасувати

# Toast
toast-undo = Скасувати

# Menu - Edit
menu-edit = Редагувати
menu-undo = Скасувати дію
menu-redo = Повторити дію
menu-revert-all = Скасувати всі зміни
menu-cut = Вирізати
menu-copy = Копіювати
menu-paste = Вставити
menu-copy-to-clipboard = Копіювати зображення до буфера обміну
menu-copy-file-path = Копіювати шлях до файлу
menu-rotate-left = Обернути ліворуч на 90°
menu-rotate-right = Обернути праворуч на 90°

# Menu - View
menu-view = Вид
menu-zoom-in = Збільшити
menu-zoom-out = Зменшити
menu-fit-to-view = Вписати у вікно
menu-actual-size = Справжній розмір
menu-fullscreen = Повний екран

# Toolbar
toolbar-crop = Обрізати
toolbar-annotate = Позначки
toolbar-select = Вибрати
toolbar-apply = Застосувати
toolbar-cancel = Скасувати
toolbar-rotate-left = Обернути ліворуч
toolbar-rotate-right = Обернути праворуч
toolbar-zoom-in = Збільшити
toolbar-zoom-out = Зменшити
toolbar-fullscreen = Повний екран
toolbar-save = Зберегти
toolbar-save-as = Зберегти як

# Unsaved edits dialog
unsaved-dialog-title = Незбережені зміни
unsaved-dialog-body = У зображенні є незбережені зміни. Зберегти їх, перш ніж продовжити?
unsaved-save = Зберегти
unsaved-save-as = Зберегти як
unsaved-discard = Не зберігати
unsaved-cancel = Скасувати

# Shapes Tool
shapes-rectangle = Прямокутник
shapes-block = Прямокутник із заливкою
shapes-line = Лінія
shapes-arrow = Стрілка
shapes-ellipse = Еліпс
shapes-polygon = Багатокутник
shapes-star = Зірка

# Text tool
text-tool = Напис

# Drawing Tool
drawing-highlighter = Маркер
drawing-pen = Ручка
drawing-pencil = Олівець

# Crop Tool
crop-custom = Довільне
crop-original = Оригінал
crop-1-1 = 1:1
crop-16-9 = 16:9
crop-7-5 = 7:5
crop-5-7 = 5:7
crop-4-3 = 4:3
crop-3-4 = 3:4
crop-3-2 = 3:2
crop-2-3 = 2:3

# Image Details Labels
image-type = Тип:
image-file-size = Розмір файлу:
image-created = Створено:
image-modified = Змінено:
image-accessed = Дата доступу:
image-size = Розміри зображення:
image-dpi = Роздільна здатність:
image-folder = Тека:

# Settings
settings = Налаштування
appearance = Вигляд
theme = Тема
match-desktop = Системна
dark = Темна
light = Світла
startup = Запуск
show-navbar = Показувати панель мініатюр під час запуску

type-here = Введіть текст...

# WMDE additions: interface strings upstream keeps outside fl!()
no-open-file = Файл не відкрито
no-image-loaded = Зображення не відкрито
image-dpi-value = { $dpi } пікс./дюйм
color-reset = Скинути до типового
color-recent = Нещодавні кольори
color-copied = Скопійовано!
no-image-selected = Зображення не вибрано
dialog-open-image = Відкрити зображення
dialog-open-folder = Відкрити теку
dialog-save-image-as = Зберегти зображення як...
filter-all = Всі зображення
unnamed-file = файл
moved-to-trash = «{ $file }» пересунуто до смітника
image-type-format = Зображення { $format }
image-type-generic = Зображення
size-b = Б
size-kb = КБ
size-mb = МБ
size-gb = ГБ
size-bytes = ({ $bytes } { $count ->
    [one] байт
    [few] байти
    [many] байтів
   *[other] байта
})
# chrono strftime pattern for the created / modified / accessed dates
date-time-format = %d.%m.%Y %H:%M:%S
# WMDE: failures, shown as a toast or in place of the picture
error-open-title = Не вдалося відкрити «{ $file }»
error-open = Не вдалося відкрити «{ $file }»: { $error }
error-save = Не вдалося зберегти зображення: { $error }
error-open-folder = Не вдалося відкрити теку
error-open-link = Не вдалося відкрити посилання
error-dialog = Не вдалося показати діалог вибору файлу
error-wallpaper = Не вдалося зробити зображення тлом: { $error }
error-trash = Не вдалося пересунути «{ $file }» до смітника: { $error }
error-restore = Не вдалося відновити зі смітника: { $error }
# WMDE: the reason put after the colon of a failure
error-reason-no-image = зображення не відкрито
error-reason-still-loading = зображення ще завантажується
error-reason-not-in-trash = файлу вже немає у смітнику
error-reason-not-found = файл не знайдено
error-reason-permission = немає доступу
error-reason-read-only = файлова система лише для читання
error-reason-no-space = на диску немає місця
error-reason-unreadable = файл пошкоджено або його формат не підтримується
error-reason-unwritable = зображення не можна записати в цьому форматі
error-reason-name-taken = файл із такою назвою вже існує
error-reason-system = сталася системна помилка
# WMDE: the details drawer, grouped and worded as in wmde-photos
image-group-file = Файл
image-group-picture = Зображення
image-group-shot = Знімок
image-group-place = Місце
image-group-other = Інше
image-name = Назва:
image-megapixels = Мегапікселів:
image-taken = Знято:
image-camera = Камера:
image-lens = Обʼєктив:
image-exposure = Витримка:
image-exposure-value = { $time } с
image-aperture = Діафрагма:
image-iso = ISO:
image-focal-length = Фокусна відстань:
image-focal-length-value = { $mm } мм
image-flash = Спалах:
image-flash-fired = Спрацював
image-flash-not-fired = Не спрацював
image-flash-absent = Немає
image-flash-forced = примусово
image-flash-suppressed = вимкнено
image-flash-auto = авто
image-flash-red-eye = усунення червоних очей
image-latitude = Широта:
image-latitude-value = { $degrees }° { $hemisphere ->
    [north] пн. ш.
   *[south] пд. ш.
}
image-longitude = Довгота:
image-longitude-value = { $degrees }° { $hemisphere ->
    [east] сх. д.
   *[west] зх. д.
}
image-software = Програма:
image-author = Автор:
image-description = Опис:
image-copyright = Права:
image-no-exif = Знімок не містить даних EXIF
# chrono strftime pattern for the camera's clock, which records no time zone
date-time-format-camera = %d.%m.%Y %H:%M:%S
