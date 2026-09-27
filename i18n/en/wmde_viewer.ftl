# Application
app-name = WMDE Viewer
app-description = Image viewer for the WMDE desktop

## About
menu-about = About WMDE Viewer...
repository = Repository
support = Support

# Menu - File
menu-file = File
menu-open-file = Open file
menu-open-recent = Open recent
menu-open-folder = Open folder
menu-open-containing = Open containing folder
menu-save = Save
menu-save-as = Save as
menu-set-wallpaper = Set as wallpaper
menu-move-to-trash = Move to trash
menu-delete-permanently = Delete permanently
menu-image-details = Image details
menu-quit = Quit

# Wallpaper dialog
wallpaper-dialog-title = Set as wallpaper
wallpaper-all-displays = All displays
wallpaper-cancel = Cancel

# Toast
toast-undo = Undo

# Menu - Edit
menu-edit = Edit
menu-undo = Undo
menu-redo = Redo
menu-revert-all = Revert all changes
menu-cut = Cut
menu-copy = Copy
menu-paste = Paste
menu-copy-to-clipboard = Copy image to clipboard
menu-copy-file-path = Copy file path
menu-rotate-left = Rotate left 90°
menu-rotate-right = Rotate right 90°

# Menu - View
menu-view = View
menu-zoom-in = Zoom in
menu-zoom-out = Zoom out
menu-fit-to-view = Fit to view
menu-actual-size = Actual Size
menu-fullscreen = Fullscreen

# Toolbar
toolbar-crop = Crop
toolbar-annotate = Markup
toolbar-select = Select
toolbar-apply = Apply
toolbar-cancel = Cancel
toolbar-rotate-left = Rotate Left
toolbar-rotate-right = Rotate Right
toolbar-zoom-in = Zoom in
toolbar-zoom-out = Zoom out
toolbar-fullscreen = Fullscreen
toolbar-save = Save
toolbar-save-as = Save as

# Unsaved edits dialog
unsaved-dialog-title = Unsaved edits
unsaved-dialog-body = You have unsaved edits to this image. Save them before continuing?
unsaved-save = Save
unsaved-save-as = Save as
unsaved-discard = Discard
unsaved-cancel = Cancel

# Shapes Tool
shapes-rectangle = Rectangle
shapes-block = Block
shapes-line = Line
shapes-arrow = Arrow
shapes-ellipse = Ellipse
shapes-polygon = Polygon
shapes-star = Star

# Text tool
text-tool = Text

# Drawing Tool
drawing-highlighter = Highlighter
drawing-pen = Pen
drawing-pencil = Pencil

# Crop Tool
crop-custom = Custom
crop-original = Original
crop-1-1 = 1:1
crop-16-9 = 16:9
crop-7-5 = 7:5
crop-5-7 = 5:7
crop-4-3 = 4:3
crop-3-4 = 3:4
crop-3-2 = 3:2
crop-2-3 = 2:3

# Image Details Labels
image-type = Type:
image-file-size = Size:
image-created = Created:
image-modified = Modified:
image-accessed = Accessed:
image-size = Image size:
image-dpi = Image DPI:
image-folder = Folder:

# Settings
settings = Settings
appearance = Appearance
theme = Theme
match-desktop = Match desktop
dark = Dark
light = Light
startup = Startup
show-navbar = Show navigation bar on launch

type-here = Type here...

# WMDE additions: interface strings upstream keeps outside fl!()
no-open-file = No Open File
no-image-loaded = No image loaded
image-dpi-value = { $dpi } pixels/inch
color-reset = Reset to default
color-recent = Recent colors
color-copied = Copied!
no-image-selected = No image selected
dialog-open-image = Open Image
dialog-open-folder = Open Folder
dialog-save-image-as = Save Image As...
filter-all = All
unnamed-file = file
moved-to-trash = Moved { $file } to trash
image-type-format = { $format } image
image-type-generic = Image
size-b = B
size-kb = KB
size-mb = MB
size-gb = GB
size-bytes = ({ $bytes } { $count ->
    [one] byte
   *[other] bytes
})
# chrono strftime pattern for the created / modified / accessed dates
date-time-format = %a %d %b %Y %I:%M:%S %p %Z
# WMDE: failures, shown as a toast or in place of the picture
error-open-title = Could not open { $file }
error-open = Could not open { $file }: { $error }
error-save = Could not save the image: { $error }
error-open-folder = Could not open the folder
error-open-link = Could not open the link
error-dialog = Could not show the file dialog
error-wallpaper = Could not set the wallpaper
error-trash = Could not move { $file } to trash: { $error }
error-restore = Could not restore from trash: { $error }
# WMDE: the reason put after the colon of a failure
error-reason-no-image = no image is open
error-reason-still-loading = the picture is still loading
error-reason-not-in-trash = the file is no longer in the trash
error-reason-not-found = the file was not found
error-reason-permission = access denied
error-reason-read-only = the file system is read-only
error-reason-no-space = no space left on the disk
error-reason-unreadable = the file is damaged or its format is not supported
error-reason-unwritable = the image cannot be written in this format
error-reason-name-taken = a file with this name already exists
error-reason-system = a system error occurred
