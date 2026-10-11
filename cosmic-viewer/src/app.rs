// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    details, failure, fl, icon_cache_get,
    key_binds::{self, MenuAction, keyboard_shortcut_handler},
    menu::menu_bar,
    message::{
        ContextMessage, EditMessage, ImageMessage, NavMessage, UnsavedChoice, ViewerMessage,
    },
    watcher::WatcherEvent,
};
use cosmic::{
    Action, Application, ApplicationExt, Core, Element, Task, Theme,
    app::context_drawer,
    cosmic_config::CosmicConfigEntry,
    dialog::file_chooser::{self, FileFilter},
    iced::{
        self, Alignment, Background, Border, Color, Length, Point, Rectangle, Size, Subscription,
        Vector,
        advanced::graphics::text::{cosmic_text, font_system},
        alignment::{Horizontal, Vertical},
        clipboard, event, font,
        keyboard::{
            Modifiers,
            key::{Key, Named},
        },
        widget::{
            grid,
            scrollable::{AbsoluteOffset, scroll_to},
            sensor, stack,
        },
        window,
    },
    task::{self, future},
    theme::{self, Button},
    widget::{
        self, Column, Id, Row, Space, Toasts,
        about::About,
        button::{self, focus},
        canvas,
        color_picker::{
            self,
            ColorPickerUpdate::{self, AppliedColor, Cancel, ToggleColorPicker},
        },
        container, divider, dropdown, icon,
        image::Handle,
        menu::{KeyBind, menu_button},
        mouse_area, nav_bar, popover, scrollable, segmented_button, segmented_control,
        space::horizontal,
        text, toaster,
    },
};
use image::DynamicImage;
use std::{
    collections::{HashMap, HashSet},
    fmt::Write as _,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use viewer_canvas::{CanvasImage, CanvasMessage, ToolKind, ViewportManager};
use viewer_config::{AppTheme, ViewerConfig};
use viewer_core::{
    CachedImage, ClipboardImage, ImageCache, NavState, get_image_dir, image_mime_type, is_drawing,
    is_supported_image, keeps_depth, load_image, load_thumbnail, nav, raw::raw_mime_type,
    read_details, save_image, scan_dir, sort_files,
};
use viewer_toolbar::{ItemPriority, ToolbarItem, ToolbarMode, responsive_toolbar};
use viewer_tools::{
    FONT_SIZE_PRESETS_PT, ToolOperation,
    annotate::{
        AnnotateColor, AnnotateTool, HighlighterPreview, PenPreview, ShapeKind, ShapePreview,
        TextDragHandle, TextOperation, TextPreview,
        tool::{FONT_SIZE_LABELS, pt_to_px},
    },
    crop::{CropOperation, CropRatio, CropSelection},
    rotate::{RotateDirection, RotateOperation},
};
use viewer_widgets::dashed_shape::DashedBorder;

/// A deferred action that leaves the current (dirty) image, held until the
/// unsaved-edits prompt is resolved. `Replay` re-dispatches the original message
/// once the edits are saved or discarded; `Quit` closes the window.
enum PendingAction {
    Replay(Box<ViewerMessage>),
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextStyle {
    Bold,
    Italic,
    Underline,
}

// reason: independent UI toggle flags (text styles, popups, fullscreen, prefs);
// grouping into a bitflags type would obscure their distinct meanings.
#[allow(clippy::struct_excessive_bools)]
pub struct CosmicViewer {
    core: Core,
    cur_scroll: Option<iced::widget::scrollable::Viewport>,
    hovered_item: Option<usize>,
    key_binds: HashMap<KeyBind, MenuAction>,
    config: ViewerConfig,
    app_themes: Vec<String>,
    nav: NavState,
    nav_bar_model: nav_bar::Model,
    cache: ImageCache,
    nav_handles: Vec<Option<Handle>>,
    scroll_id: Id,
    viewport: ViewportManager,
    context_page: Option<ContextMessage>,
    context_menu_position: Option<Point>,
    annotate_tool: AnnotateTool,
    annotate_color: AnnotateColor,
    annotate_stroke_size: f32,
    crop_ratio: CropRatio,
    crop_ratio_popup: bool,
    text_editing: bool,
    move_target: Option<usize>,
    move_start: Option<Point>,
    font_families: Vec<&'static str>,
    text_font_family: &'static str,
    text_font_index: Option<usize>,
    text_font_size: f32,
    text_bold: bool,
    text_italic: bool,
    text_underline: bool,
    text_alignment: Horizontal,
    text_style_model: segmented_button::MultiSelectModel,
    text_align_model: segmented_button::SingleSelectModel,
    show_text_format_menu: bool,
    shape_popup: bool,
    stroke_popup: bool,
    color_picker: cosmic::widget::ColorPickerModel,
    has_custom_color: bool,
    selected_shape: AnnotateTool,
    window_width: Option<f32>,
    is_fullscreen: bool,
    saved_nav_toggled: bool,
    wallpaper_dialog: Option<PathBuf>,
    delete_dialog: Option<PathBuf>,
    /// Pending leave-action awaiting the unsaved-edits prompt's resolution.
    unsaved_dialog: Option<PendingAction>,
    available_outputs: Vec<String>,
    watcher_rescan_pending: bool,
    /// WMDE: the files the album was opened with when there were several; `None` for an album
    /// of a directory. A rescan orders these again rather than listing the directory.
    chosen_files: Option<Vec<PathBuf>>,
    was_narrow: bool,
    toasts: Toasts<ViewerMessage>,
    /// WMDE: why the picture at this path did not open, shown in place of the picture.
    load_error: Option<(PathBuf, String)>,
    /// WMDE: the file whose picture the canvas holds. While the next picture loads the canvas
    /// keeps the previous one, and a save must not write it over the file now selected.
    canvas_path: Option<PathBuf>,
    /// WMDE: what the file of the described picture says about itself, for the details
    /// drawer. Read while the drawer is open, dropped when it closes.
    details: Option<(PathBuf, Box<viewer_core::Details>)>,
    /// WMDE: moved on by every change to the current picture's file, so the drawer reads the
    /// file again.
    details_stamp: u64,
    /// WMDE: wallpapers being set, and a quit waiting for them to finish.
    wallpapers_running: usize,
    quit_waiting: bool,
    about: About,
}

impl CosmicViewer {
    /// The current image has committed edits (crop/rotate/annotations) not yet saved.
    fn has_unsaved_edits(&self) -> bool {
        !self.viewport.operations().is_empty()
    }

    /// If the current image is dirty, stash `replay` behind the unsaved-edits prompt
    /// and return `true` (caller should abort). Otherwise return `false` (proceed).
    fn guard_unsaved(&mut self, replay: ViewerMessage) -> bool {
        if self.has_unsaved_edits() {
            self.unsaved_dialog = Some(PendingAction::Replay(Box::new(replay)));
            true
        } else {
            false
        }
    }

    /// Carry out the action deferred by the prompt once edits are saved/discarded.
    fn resolve_pending(&mut self) -> Task<Action<ViewerMessage>> {
        match self.unsaved_dialog.take() {
            Some(PendingAction::Replay(msg)) => self.update(*msg),
            Some(PendingAction::Quit) => self.update(ViewerMessage::Quit),
            None => Task::none(),
        }
    }

    // WMDE: an action that failed says so twice, in the journal and as a toast. A button
    // that silently did nothing is not acceptable.
    fn complain(
        &mut self,
        action: &str,
        error: &str,
        message: String,
    ) -> Task<Action<ViewerMessage>> {
        tracing::error!("cannot {action}: {error}");
        self.show_error(message)
    }

    fn show_error(&mut self, message: String) -> Task<Action<ViewerMessage>> {
        let toast = toaster::Toast::new(message).duration(toaster::Duration::Long);
        self.toasts.push(toast).map(Action::App)
    }

    /// Write the edited picture to `path`. `Err` carries the toast that says why it was not.
    /// `in_place` is a save over the picture's own file.
    fn write_edits(
        &mut self,
        path: &Path,
        in_place: bool,
    ) -> Result<(), Task<Action<ViewerMessage>>> {
        if let Some(current) = self.nav.current().cloned()
            && self.canvas_path.as_ref() != Some(&current)
        {
            // The canvas holds another picture while this one loads, or none at all.
            let reason = if self.canvas_path.is_some() || self.cache.is_pending(&current) {
                fl!("error-reason-still-loading")
            } else if let Some((_, reason)) = self
                .load_error
                .as_ref()
                .filter(|(failed, _)| failed == &current)
            {
                reason.clone()
            } else {
                fl!("error-reason-no-image")
            };
            let detail = format!("the canvas does not hold {}", current.display());
            return Err(self.complain(
                "save the image",
                &detail,
                fl!("error-save", error = reason),
            ));
        }
        let Some(image) = self.flatten_image() else {
            return Err(self.complain(
                "save the image",
                "no image is open",
                fl!("error-save", error = fl!("error-reason-no-image")),
            ));
        };
        // WMDE: a save over the file itself does not quietly lower its depth, as the AVIF
        // encoder does with a 10-bit AVIF; "Save as" leaves the format to the person.
        if in_place && !keeps_depth(&image, path) {
            return Err(self.complain(
                &format!("save {}", path.display()),
                &format!(
                    "the format keeps fewer bits than the {:?} picture",
                    image.color()
                ),
                fl!("error-save", error = fl!("error-reason-depth")),
            ));
        }
        save_image(&image, path).map_err(|err| {
            self.complain(
                &format!("save {}", path.display()),
                &err.to_string(),
                fl!("error-save", error = failure::save_reason(&err)),
            )
        })
    }

    /// Put the full picture of `path` from the cache on the canvas.
    fn show_cached(&mut self, path: PathBuf) {
        if let Some(cached) = self.cache.get_full(&path) {
            self.viewport.set_image(
                Some(CanvasImage {
                    handle: cached.handle,
                    width: cached.width,
                    height: cached.height,
                }),
                Some(cached.image),
            );
            self.canvas_path = Some(path);
        }
    }

    /// Empty the canvas and leave nothing half-edited on it: the edits, the tool, the text being
    /// typed, the colour picker and the text format menu go with the picture.
    fn clear_canvas(&mut self) {
        self.viewport.cancel_tool();
        self.viewport.revert_all();
        self.viewport.set_image(None, None);
        self.canvas_path = None;
        self.reset_editing();
    }

    /// Drop the editing state that points into the edits: after `revert_all` a selected or
    /// moved annotation index would point past the end of the empty list.
    fn reset_editing(&mut self) {
        self.viewport.move_mode = false;
        self.viewport.select_target = None;
        self.move_target = None;
        self.move_start = None;
        self.text_editing = false;
        self.show_text_format_menu = false;
        if self.color_picker.get_is_active() {
            _ = self
                .color_picker
                .update::<ViewerMessage>(ColorPickerUpdate::ToggleColorPicker);
        }
    }

    /// Save the edited picture over its own file and reload it. `Err` carries the toast; the
    /// edits are kept then.
    fn save_in_place(
        &mut self,
    ) -> Result<Task<Action<ViewerMessage>>, Task<Action<ViewerMessage>>> {
        let Some(path) = self.nav.current().cloned() else {
            return Err(self.complain(
                "save the image",
                "no image is open",
                fl!("error-save", error = fl!("error-reason-no-image")),
            ));
        };
        self.write_edits(&path, true)?;
        // WMDE: the details drawer reads the file again: a re-encoded file has lost its EXIF.
        // The watcher's event may name the file otherwise (a relative path, a link's target).
        self.details_stamp = self.details_stamp.wrapping_add(1);
        self.viewport.revert_all();
        self.cache.remove_full(&path);
        self.viewport.cancel_tool();
        self.text_editing = false;
        Ok(self.load_full_image(path))
    }

    // WMDE: the background through wmde-bg's own settings crate (`viewer_core::set_wallpaper`)
    // instead of RON written by hand, off the drawing thread: a picture wmde-bg cannot read is
    // decoded and written as a PNG first. Requests go in turn, and a later one supersedes one
    // still waiting, so the last picture asked for is the one on the desktop. A plain thread, as
    // for OpenContaining: a file on a dead mount must not hold the exit.
    fn wallpaper_task(
        &mut self,
        path: PathBuf,
        output: Option<String>,
    ) -> Task<Action<ViewerMessage>> {
        self.wallpapers_running += 1;
        let request = viewer_core::wallpaper::next_request();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let result =
                viewer_core::wallpaper::set_wallpaper_in_turn(request, &path, output.as_deref())
                    .map(drop)
                    .map_err(|err| {
                        (
                            failure::wallpaper_reason(&err),
                            format!("{}: {err}", path.display()),
                        )
                    });
            _ = sender.send(result);
        });
        future(async move {
            let result = receiver
                .await
                .unwrap_or_else(|err| Err((fl!("error-reason-system"), err.to_string())));
            Action::App(ViewerMessage::WallpaperResult(result))
        })
    }

    fn is_narrow(&self) -> bool {
        // Tie "narrow" to libcosmic's own condensed breakpoint (the point it would hide the main
        // content, ~360px content width). A separate, narrower threshold left a gap band where the
        // framework hid the content but the app still drew the nav as a fixed column, so the main
        // view vanished (#32).
        self.core().is_condensed()
    }

    fn save_last_color(&mut self) {
        let c = self.annotate_color.0;
        self.config.last_color = Some([c.r, c.g, c.b, c.a]);
        if let Ok(config_handle) = viewer_config::config() {
            let _ = self.config.write_entry(&config_handle);
        }
    }

    fn reload_image_list(&self) -> Task<Action<ViewerMessage>> {
        let include_hidden = self.config.show_hidden_files;
        let sort_mode = self.config.sort_mode;
        let sort_order = self.config.sort_order;

        if let Some(files) = self.chosen_files.clone() {
            return future(async move {
                let images = sort_files(files, sort_mode, sort_order).await;
                Action::App(ViewerMessage::Nav(NavMessage::DirectoryRefreshed(images)))
            });
        }

        let dir = self.nav.current().map_or_else(
            || self.nav.dir().map(std::path::Path::to_path_buf),
            |current| get_image_dir(current),
        );

        if let Some(dir) = dir {
            return future(async move {
                let images = scan_dir(&dir, include_hidden, sort_mode, sort_order).await;
                Action::App(ViewerMessage::Nav(NavMessage::DirectoryRefreshed(images)))
            });
        }

        Task::none()
    }

    /// WMDE: what the command line asked for. One path opens as a path does; several paths with
    /// files among them make an album of those files alone, ordered as the album orders, the
    /// first given shown. A directory or a missing path among them is left out.
    fn open_start(&mut self, paths: Vec<PathBuf>) -> Task<Action<ViewerMessage>> {
        let files = given_files(&paths);
        if paths.len() < 2 || files.is_empty() {
            let mut paths = paths.into_iter();
            let first = paths.next();
            for other in paths {
                tracing::warn!("{} is left out: only the first path opens", other.display());
            }
            return first.map_or_else(Task::none, |path| self.open_path(path));
        }
        for other in paths.iter().filter(|path| !files.contains(path)) {
            tracing::warn!(
                "{} is left out of the album of the files given",
                other.display()
            );
        }
        let first = files[0].clone();
        let dir = first.parent().map_or_else(PathBuf::new, Path::to_path_buf);
        self.chosen_files = Some(files.clone());
        let sort_mode = self.config.sort_mode;
        let sort_order = self.config.sort_order;
        future(async move {
            let images = sort_files(files, sort_mode, sort_order).await;
            Action::App(ViewerMessage::Nav(NavMessage::ScanComplete(
                dir,
                images,
                Some(first),
            )))
        })
    }

    /// WMDE: the directories the watcher follows: each one a file of the album given on the
    /// command line lives in, else the album's own.
    fn watched_dirs(&self) -> Vec<PathBuf> {
        self.chosen_files.as_ref().map_or_else(
            || self.nav.dir().map(Path::to_path_buf).into_iter().collect(),
            |files| {
                let mut dirs: Vec<PathBuf> = files
                    .iter()
                    .filter_map(|file| file.parent().map(Path::to_path_buf))
                    .collect();
                dirs.sort();
                dirs.dedup();
                dirs
            },
        )
    }

    fn open_path(&mut self, path: PathBuf) -> Task<Action<ViewerMessage>> {
        // WMDE: any path opened from here on is an album of its own directory.
        self.chosen_files = None;
        if let Some(dir) = get_image_dir(&path) {
            let select = path.is_file().then_some(path);
            let include_hidden = self.config.show_hidden_files;
            let sort_mode = self.config.sort_mode;
            let sort_order = self.config.sort_order;
            let dir_clone = dir.clone();

            let title = match select.as_ref() {
                Some(tab) => tab
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default(),
                None => fl!("no-open-file"),
            };

            let window_title = format!("{title} - {}", fl!("app-name"));
            Task::batch([
                if let Some(window_id) = self.core.main_window_id() {
                    self.set_window_title(window_title, window_id)
                } else {
                    Task::none()
                },
                future(async move {
                    let images = scan_dir(&dir, include_hidden, sort_mode, sort_order).await;
                    Action::App(ViewerMessage::Nav(NavMessage::ScanComplete(
                        dir_clone, images, select,
                    )))
                }),
            ])
        } else if path.is_file()
            && is_supported_image(&path)
            && path
                .file_name()
                .is_some_and(|name| !name.to_str().is_some_and(|name| name.starts_with('.')))
        {
            task::message(Action::App(ViewerMessage::Nav(NavMessage::ScanComplete(
                path.parent().unwrap().to_path_buf(),
                vec![path.clone()],
                Some(path),
            ))))
        } else if !path.exists() {
            // WMDE: a path that is not there is the picture that did not open, with its reason
            // in its place, not an empty window with nothing said.
            let dir = path.parent().map_or_else(PathBuf::new, Path::to_path_buf);
            task::message(Action::App(ViewerMessage::Nav(NavMessage::ScanComplete(
                dir,
                vec![path.clone()],
                Some(path),
            ))))
        } else {
            Task::none()
        }
    }

    // reason: thumbnail size and grid index are small in-range counts; exact as f32.
    #[allow(clippy::cast_precision_loss)]
    fn grid_scroll_offset(&self, idx: usize) -> f32 {
        let thumb_size = self.config.thumbnail_size.pixels() as f32;
        let button_padding = 8.0;
        let cell_size = thumb_size + (button_padding * 2.0);
        let row_spacing = 8.0;
        idx as f32 * (cell_size + row_spacing)
    }

    fn rebuild_nav_handles(&mut self) {
        self.nav_handles = self
            .nav
            .images()
            .iter()
            .map(|path| self.cache.get_thumbnail(path))
            .collect();
    }

    /// Rebuild the working image.
    /// Used after dimension-changing operations like crop and rotate.
    fn rebuild_working_image(&mut self) {
        let original = self
            .nav
            .current()
            .and_then(|path| self.cache.get_full(path))
            .map(|cached| cached.image);

        // WMDE: the cached original is shared, and each operation makes a new picture from it;
        // with none, the canvas shares the original's pixels.
        if let Some(mut base) = original {
            for op in self.viewport.operations() {
                if let Some(rotate) = op.as_any().downcast_ref::<RotateOperation>() {
                    base = Arc::new(match rotate.direction {
                        RotateDirection::Left => base.rotate270(),
                        RotateDirection::Right => base.rotate90(),
                    });
                } else if let Some(crop) = op.as_any().downcast_ref::<CropOperation>() {
                    base = Arc::new(crop_to_region(&base, crop.region));
                }
            }

            self.viewport.replace_working_image(base);

            self.viewport.rebuild_display();
        }
    }

    fn load_nearby_thumbnails(&self, center: usize, radius: usize) -> Task<Action<ViewerMessage>> {
        let max_size = self.config.thumbnail_size.pixels();
        let images = self.nav.images();
        let total = images.len();

        let start = center.saturating_sub(radius);
        let end = (center + radius + 1).min(total);

        let tasks: Vec<_> = images[start..end]
            .iter()
            .filter(|path| {
                self.cache.get_thumbnail(path).is_none() && !self.cache.is_thumbnail_pending(path)
            })
            .map(|path| {
                self.cache.set_thumbnail_pending(path.clone());
                let cache = self.cache.clone();
                let path = path.clone();
                future(async move {
                    match load_thumbnail(path.clone(), max_size).await {
                        Ok(loaded) => {
                            cache.insert_thumbnail(loaded.path.clone(), loaded.handle);
                            Action::App(ViewerMessage::Image(ImageMessage::ThumbnailReady(
                                loaded.path,
                                loaded.width,
                                loaded.height,
                            )))
                        }
                        Err(_) => {
                            Action::App(ViewerMessage::Image(ImageMessage::ThumbnailError(path)))
                        }
                    }
                })
            })
            .collect();

        if tasks.is_empty() {
            Task::none()
        } else {
            Task::batch(tasks)
        }
    }

    fn load_full_image(&mut self, path: PathBuf) -> Task<Action<ViewerMessage>> {
        // WMDE: the title names the current picture, and names it now. It was set once each
        // load ended, neighbours loaded ahead of time included, so the window was titled after
        // whichever load finished last.
        let window_title = match self.core.main_window_id() {
            Some(window_id) if self.nav.current() == Some(&path) => {
                let title = format!(
                    "{} - {}",
                    path.file_name()
                        .map(|f| f.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    fl!("app-name")
                );
                self.set_window_title(title, window_id)
            }
            _ => Task::none(),
        };
        if self.cache.contains_full(&path) || self.cache.is_pending(&path) {
            return window_title;
        }

        self.cache.set_pending(path.clone());
        let cache = self.cache.clone();

        let load = future(async move {
            match load_image(path.clone()).await {
                Ok(loaded) => {
                    cache.insert_full(
                        loaded.path.clone(),
                        CachedImage {
                            handle: loaded.handle,
                            image: Arc::new(loaded.image),
                            width: loaded.width,
                            height: loaded.height,
                        },
                    );
                    Action::App(ViewerMessage::Image(ImageMessage::ImageReady(loaded.path)))
                }
                Err(err) => Action::App(ViewerMessage::Image(ImageMessage::LoadError {
                    path,
                    reason: failure::load_reason(&err),
                    detail: err.to_string(),
                })),
            }
        });
        Task::batch([window_title, load])
    }

    fn image_details_page(&self) -> Element<'_, ViewerMessage> {
        let spacing = cosmic::theme::active().cosmic().spacing;
        let mut content = Column::new().spacing(spacing.space_m);

        let Some(path) = self.nav.current() else {
            return content.push(text::body(fl!("no-image-loaded"))).into();
        };

        // WMDE: the rows of wmde-photos' details panel in its groups, with the dates and the
        // folder upstream shows as well. What has to be read from the file arrives with
        // `DetailsRead`; until then only what needs no reading is shown. A row without a value
        // is left out, and so is a group left with no rows.
        let details = self
            .details
            .as_ref()
            .filter(|(read, _)| read == path)
            .map(|(_, details)| details.as_ref());
        let photo = details.and_then(|details| details.exif.as_ref());
        // An SVG is rendered at the texture size; its size is the one the drawing declares.
        let size = if is_drawing(path) {
            details.and_then(|d| d.drawing_size)
        } else {
            self.cache.full_size(path)
        };
        let name = |name: &std::ffi::OsStr| name.to_string_lossy().into_owned();

        let groups = [
            (
                fl!("image-group-file"),
                vec![
                    (fl!("image-name"), path.file_name().map(name)),
                    (
                        fl!("image-file-size"),
                        details.and_then(|d| d.size).map(format_file_size),
                    ),
                    (
                        fl!("image-folder"),
                        path.parent().and_then(Path::file_name).map(name),
                    ),
                    (
                        fl!("image-created"),
                        details.and_then(|d| d.created).map(format_system_time),
                    ),
                    (
                        fl!("image-modified"),
                        details.and_then(|d| d.modified).map(format_system_time),
                    ),
                    (
                        fl!("image-accessed"),
                        details.and_then(|d| d.accessed).map(format_system_time),
                    ),
                ],
            ),
            (
                fl!("image-group-picture"),
                vec![
                    (fl!("image-size"), size.map(|(w, h)| format!("{w} x {h}"))),
                    (
                        fl!("image-megapixels"),
                        size.map(|(w, h)| details::megapixels(w, h)),
                    ),
                    // The content's format; the name's only when the content is not recognised.
                    (
                        fl!("image-type"),
                        details.map(|d| {
                            d.format.map_or_else(
                                || {
                                    friendly_type_name(
                                        path.extension()
                                            .and_then(|ext| ext.to_str())
                                            .unwrap_or_default(),
                                    )
                                },
                                |format| fl!("image-type-format", format = format),
                            )
                        }),
                    ),
                    (
                        fl!("image-dpi"),
                        details
                            .and_then(|d| d.dpi)
                            .map(|dpi| fl!("image-dpi-value", dpi = dpi)),
                    ),
                ],
            ),
            (
                fl!("image-group-shot"),
                vec![
                    (
                        fl!("image-taken"),
                        photo.and_then(|p| p.taken.as_ref()).map(details::taken),
                    ),
                    (fl!("image-camera"), photo.and_then(|p| p.camera.clone())),
                    (fl!("image-lens"), photo.and_then(|p| p.lens.clone())),
                    (
                        fl!("image-exposure"),
                        photo.and_then(|p| p.exposure).map(details::exposure),
                    ),
                    (
                        fl!("image-aperture"),
                        photo.and_then(|p| p.aperture).map(details::aperture),
                    ),
                    (
                        fl!("image-iso"),
                        photo.and_then(|p| p.iso).map(|iso| iso.to_string()),
                    ),
                    (
                        fl!("image-focal-length"),
                        photo
                            .and_then(|p| p.focal_length)
                            .map(details::focal_length),
                    ),
                    (
                        fl!("image-flash"),
                        photo.and_then(|p| p.flash).map(details::flash),
                    ),
                ],
            ),
            (
                fl!("image-group-place"),
                vec![
                    (
                        fl!("image-latitude"),
                        photo.and_then(|p| p.latitude).map(details::latitude),
                    ),
                    (
                        fl!("image-longitude"),
                        photo.and_then(|p| p.longitude).map(details::longitude),
                    ),
                ],
            ),
            (
                fl!("image-group-other"),
                vec![
                    (
                        fl!("image-software"),
                        photo.and_then(|p| p.software.clone()),
                    ),
                    (fl!("image-author"), photo.and_then(|p| p.author.clone())),
                    (
                        fl!("image-description"),
                        photo.and_then(|p| p.description.clone()),
                    ),
                    (
                        fl!("image-copyright"),
                        photo.and_then(|p| p.copyright.clone()),
                    ),
                ],
            ),
        ];

        for (title, rows) in groups {
            let rows: Vec<_> = rows
                .into_iter()
                .filter_map(|(label, value)| value.map(|value| detail_row(label, value)))
                .collect();
            if rows.is_empty() {
                continue;
            }
            let group = Column::new()
                .spacing(spacing.space_xs)
                .push(text::heading(title))
                .extend(rows);
            content = content.push(group);
        }

        if details.is_some_and(|d| d.exif.is_none()) {
            content = content.push(text::caption(fl!("image-no-exif")));
        }

        content.into()
    }

    fn build_context_menu_element<'a>() -> Element<'a, ViewerMessage> {
        let menu_item = |label: String, message: ViewerMessage| {
            menu_button(vec![text(label).into()]).on_press(message)
        };

        container(
            Column::new()
                .push(menu_item(
                    fl!("menu-copy-to-clipboard"),
                    ViewerMessage::CopyToClipboard,
                ))
                .push(menu_item(
                    fl!("menu-copy-file-path"),
                    ViewerMessage::CopyFilePath,
                ))
                .push(menu_item(
                    fl!("menu-revert-all"),
                    ViewerMessage::Edit(EditMessage::RevertAll),
                ))
                .push(menu_item(
                    fl!("menu-image-details"),
                    ViewerMessage::Context(ContextMessage::ImageDetails),
                ))
                .push(menu_item(
                    fl!("menu-set-wallpaper"),
                    ViewerMessage::SetWallpaper,
                ))
                .push(menu_item(
                    fl!("menu-move-to-trash"),
                    ViewerMessage::MoveToTrash,
                )),
        )
        .padding(1)
        .style(|theme| {
            let cosmic = theme.cosmic();
            // force opaque because it is not a real popup.
            let component = &cosmic.background(false).component;
            container::Style {
                icon_color: Some(component.on.into()),
                text_color: Some(component.on.into()),
                background: Some(Background::Color(component.base.into())),
                border: Border {
                    radius: cosmic.radius_s().map(|x| x + 1.0).into(),
                    width: 1.0,
                    color: component.divider.into(),
                },
                ..Default::default()
            }
        })
        .width(Length::Fixed(240.0))
        .into()
    }

    fn popup_style(theme: &cosmic::Theme) -> container::Style {
        let cosmic = theme.cosmic();
        // force opaque because it is not a real popup.
        let component = &cosmic.background(false).component;
        container::Style {
            background: Some(Background::Color(component.base.into())),
            border: Border {
                radius: cosmic.radius_s().map(|x| x + 1.0).into(),
                width: 1.0,
                color: component.divider.into(),
            },
            ..Default::default()
        }
    }

    fn build_toolbar(&self) -> Element<'_, ViewerMessage> {
        match self.viewport.active_tool() {
            Some(ToolKind::Crop) => self.build_crop_toolbar(),
            Some(ToolKind::Annotate) => self.build_annotate_toolbar(),
            _ => self.build_default_toolbar(),
        }
    }

    // reason: zoom percent is rounded, non-negative, and well within u32 range.
    // reason: one builder call per toolbar item; the flat chain reads clearer unsplit.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::too_many_lines
    )]
    fn build_default_toolbar(&self) -> Element<'_, ViewerMessage> {
        // The toolbar measures its own fit and stacks only when the single row
        // overflows; force stacking only when the window is truly condensed.
        let mode = if self.core().is_condensed() {
            ToolbarMode::Compact
        } else {
            ToolbarMode::Full
        };

        let icon_btn = |name: &'static str,
                        tooltip: String,
                        msg: ViewerMessage|
         -> Element<'_, ViewerMessage> {
            button::icon(icon_cache_get(name))
                .tooltip(tooltip)
                .on_press(msg)
                .into()
        };

        let bounds = self.viewport.last_bounds().get();
        let viewport_size = Size::new(bounds.width, bounds.height);
        let zoom_pct = format!(
            "{}%",
            self.viewport.actual_percent(viewport_size).round() as u32
        );

        responsive_toolbar(mode)
            .start(
                ToolbarItem::new(icon_btn(
                    "markup-symbolic",
                    fl!("toolbar-annotate"),
                    ViewerMessage::Edit(EditMessage::Annotate),
                ))
                .priority(ItemPriority::Essential),
            )
            .start(
                ToolbarItem::new(icon_btn(
                    "image-crop-symbolic",
                    fl!("toolbar-crop"),
                    ViewerMessage::Edit(EditMessage::Crop),
                ))
                .priority(ItemPriority::Essential),
            )
            .start(
                ToolbarItem::new(icon_btn(
                    "object-rotate-left-symbolic",
                    fl!("toolbar-rotate-left"),
                    ViewerMessage::Edit(EditMessage::RotateLeft),
                ))
                .priority(ItemPriority::Essential),
            )
            .start(
                ToolbarItem::new(icon_btn(
                    "object-rotate-right-symbolic",
                    fl!("toolbar-rotate-right"),
                    ViewerMessage::Edit(EditMessage::RotateRight),
                ))
                .priority(ItemPriority::Essential),
            )
            .center(
                // Zoom out / percentage / zoom in stay together as one atom so
                // the responsive toolbar never splits them across rows.
                ToolbarItem::new(
                    Row::new()
                        .push(icon_btn(
                            "zoom-out-symbolic",
                            fl!("toolbar-zoom-out"),
                            ViewerMessage::Canvas(CanvasMessage::ZoomOut),
                        ))
                        .push(text::body(zoom_pct))
                        .push(icon_btn(
                            "zoom-in-symbolic",
                            fl!("toolbar-zoom-in"),
                            ViewerMessage::Canvas(CanvasMessage::ZoomIn),
                        ))
                        .align_y(Alignment::Center)
                        .spacing(cosmic::theme::active().cosmic().spacing.space_xxs),
                )
                .priority(ItemPriority::Essential),
            )
            .end(
                ToolbarItem::new(icon_btn(
                    "view-actual-size-symbolic",
                    fl!("menu-actual-size"),
                    ViewerMessage::Canvas(CanvasMessage::ActualSize),
                ))
                .priority(ItemPriority::Essential),
            )
            .end(
                ToolbarItem::new(icon_btn(
                    "view-fit-symbolic",
                    fl!("menu-fit-to-view"),
                    ViewerMessage::Canvas(CanvasMessage::FitToView),
                ))
                .priority(ItemPriority::Essential),
            )
            .end(
                ToolbarItem::new(icon_btn(
                    "view-fullscreen-symbolic",
                    fl!("toolbar-fullscreen"),
                    ViewerMessage::Canvas(CanvasMessage::Fullscreen),
                ))
                .priority(ItemPriority::Essential),
            )
            .end_maybe(self.has_unsaved_edits().then(|| {
                ToolbarItem::new(
                    button::text(fl!("toolbar-save"))
                        .tooltip(fl!("toolbar-save"))
                        .on_press(ViewerMessage::Save),
                )
                .priority(ItemPriority::Essential)
            }))
            .end_maybe(self.has_unsaved_edits().then(|| {
                ToolbarItem::new(
                    button::icon(icon::from_name("document-save-as-symbolic"))
                        .tooltip(fl!("toolbar-save-as"))
                        .on_press(ViewerMessage::SaveAs),
                )
                .priority(ItemPriority::Essential)
            }))
            .view()
    }

    fn build_crop_ratio_selector(&self) -> Element<'_, ViewerMessage> {
        let trigger = button::custom(
            Row::new()
                .push(icon_cache_get("ratios-symbolic").icon().size(16))
                .push(icon_cache_get("pan-down-symbolic").icon().size(12))
                .align_y(Alignment::Center)
                .spacing(2),
        )
        .class(cosmic::theme::Button::Icon)
        .on_press(ViewerMessage::Edit(EditMessage::CropRatioPopupToggle));

        let mut pop = popover(trigger);

        if self.crop_ratio_popup {
            let is_portrait = self
                .viewport
                .image_size()
                .is_some_and(|size| size.height > size.width);

            let presets = CropRatio::presets();
            let mut list = Column::new().spacing(4);

            for ratio in presets {
                let label = match ratio {
                    CropRatio::Custom => fl!("crop-custom"),
                    CropRatio::Original => fl!("crop-original"),
                    CropRatio::Fixed(..) => ratio.label(is_portrait).to_string(),
                };
                let is_selected = *ratio == self.crop_ratio;
                let item = Row::new()
                    .push(container(text::body(label)).width(Length::Fixed(72.0)))
                    .push(if is_selected {
                        Element::from(icon(
                            icon::from_name("object-select-symbolic").size(16).into(),
                        ))
                    } else {
                        Element::from(Space::new().width(16))
                    })
                    .align_y(Alignment::Center)
                    .width(Length::Fixed(104.0))
                    .spacing(8);

                list = list
                    .push(
                        button::custom(item)
                            .class(theme::Button::Icon)
                            .width(Length::Fixed(104.0))
                            .on_press(ViewerMessage::Edit(EditMessage::CropRatio(*ratio))),
                    )
                    .width(Length::Shrink);
            }

            let popup = container(list).padding(8).style(|theme| {
                let cosmic = theme.cosmic();
                // force opaque because it is not a real popup.
                let component = &cosmic.background(false).component;
                container::Style {
                    icon_color: None,
                    text_color: None,
                    background: Some(Background::Color(component.base.into())),
                    border: Border {
                        radius: cosmic.radius_s().map(|x| x + 1.0).into(),
                        width: 1.0,
                        color: component.divider.into(),
                    },
                    ..Default::default()
                }
            });

            pop = pop
                .popup(popup)
                .position(popover::Position::Top)
                .on_close(ViewerMessage::Edit(EditMessage::CropRatioPopupToggle));
        }

        pop.into()
    }

    fn build_crop_toolbar(&self) -> Element<'_, ViewerMessage> {
        // The toolbar measures its own fit and stacks only when the single row
        // overflows; force stacking only when the window is truly condensed.
        let mode = if self.core().is_condensed() {
            ToolbarMode::Compact
        } else {
            ToolbarMode::Full
        };

        let icon_btn = |name: &'static str,
                        tooltip: String,
                        msg: ViewerMessage|
         -> Element<'_, ViewerMessage> {
            button::icon(icon_cache_get(name))
                .tooltip(tooltip)
                .on_press(msg)
                .into()
        };

        let can_undo = self.viewport.can_undo();
        let can_redo = self.viewport.can_redo();

        let undo_btn = button::icon(icon::from_name("edit-undo-symbolic"))
            .tooltip(fl!("menu-undo"))
            .on_press_maybe(can_undo.then(|| ViewerMessage::Edit(EditMessage::Undo)));

        let redo_btn = button::icon(icon::from_name("edit-redo-symbolic"))
            .tooltip(fl!("menu-redo"))
            .on_press_maybe(can_redo.then(|| ViewerMessage::Edit(EditMessage::Redo)));

        let zoom_pct = {
            let bounds = self.viewport.last_bounds().get();
            format!(
                "{:.0}%",
                self.viewport
                    .actual_percent(Size::new(bounds.width, bounds.height))
            )
        };

        responsive_toolbar(mode)
            .start(ToolbarItem::new(undo_btn))
            .start(ToolbarItem::new(redo_btn))
            .start(ToolbarItem::new(self.build_crop_ratio_selector()))
            .center(
                // Zoom out / percentage / zoom in stay together as one atom so
                // the responsive toolbar never splits them across rows.
                ToolbarItem::new(
                    Row::new()
                        .push(icon_btn(
                            "zoom-out-symbolic",
                            fl!("toolbar-zoom-out"),
                            ViewerMessage::Canvas(CanvasMessage::ZoomOut),
                        ))
                        .push(text::body(zoom_pct))
                        .push(icon_btn(
                            "zoom-in-symbolic",
                            fl!("toolbar-zoom-in"),
                            ViewerMessage::Canvas(CanvasMessage::ZoomIn),
                        ))
                        .align_y(Alignment::Center)
                        .spacing(cosmic::theme::active().cosmic().spacing.space_xxs),
                )
                .priority(ItemPriority::Essential),
            )
            .end(ToolbarItem::new(
                button::standard(fl!("toolbar-cancel"))
                    .on_press(ViewerMessage::Edit(EditMessage::CropCancel)),
            ))
            .end(ToolbarItem::new(
                button::icon(icon::from_name("object-select-symbolic"))
                    .tooltip(fl!("toolbar-apply"))
                    .on_press(ViewerMessage::Edit(EditMessage::CropApply)),
            ))
            .view()
    }

    // reason: single declarative widget tree; splitting would fragment the layout.
    #[allow(clippy::too_many_lines)]
    fn build_annotate_toolbar(&self) -> Element<'_, ViewerMessage> {
        let mode = if self.core().is_condensed() {
            ToolbarMode::Compact
        } else {
            ToolbarMode::Full
        };

        let icon_btn = |name: &'static str,
                        tooltip: String,
                        msg: ViewerMessage|
         -> button::IconButton<ViewerMessage> {
            button::icon(icon_cache_get(name))
                .tooltip(tooltip)
                .on_press(msg)
        };

        let can_undo = self.viewport.can_undo();
        let can_redo = self.viewport.can_redo();

        let undo_btn = button::icon(icon::from_name("edit-undo-symbolic"))
            .tooltip(fl!("menu-undo"))
            .on_press_maybe(can_undo.then(|| ViewerMessage::Edit(EditMessage::Undo)));

        let redo_btn = button::icon(icon::from_name("edit-redo-symbolic"))
            .tooltip(fl!("menu-redo"))
            .on_press_maybe(can_redo.then(|| ViewerMessage::Edit(EditMessage::Redo)));

        let mut toolbar = responsive_toolbar(mode)
            .start(ToolbarItem::new(undo_btn))
            .start(ToolbarItem::new(redo_btn))
            .start(ToolbarItem::new(
                divider::vertical::light().height(Length::Fixed(32.0)),
            ))
            .start(ToolbarItem::new(
                icon_btn(
                    "insert-text-symbolic",
                    fl!("text-tool"),
                    ViewerMessage::Edit(EditMessage::AnnotateTool(AnnotateTool::Text)),
                )
                .selected(self.annotate_tool == AnnotateTool::Text)
                .class(tool_toggle_class(self.annotate_tool == AnnotateTool::Text)),
            ))
            .start(ToolbarItem::new(
                icon_btn(
                    "insert-drawing-symbolic",
                    fl!("drawing-pen"),
                    ViewerMessage::Edit(EditMessage::AnnotateTool(AnnotateTool::draw_tools()[0])),
                )
                .selected(self.annotate_tool == AnnotateTool::draw_tools()[0])
                .class(tool_toggle_class(
                    self.annotate_tool == AnnotateTool::draw_tools()[0],
                )),
            ))
            .start(ToolbarItem::new(
                icon_btn(
                    "text-highlight-symbolic",
                    fl!("drawing-highlighter"),
                    ViewerMessage::Edit(EditMessage::AnnotateTool(AnnotateTool::Highlighter)),
                )
                .selected(self.annotate_tool == AnnotateTool::Highlighter)
                .class(tool_toggle_class(
                    self.annotate_tool == AnnotateTool::Highlighter,
                )),
            ))
            .start(ToolbarItem::new(self.build_shape_selector()).width_hint(120.0))
            .start_maybe({
                let has_movable =
                    self.viewport.operations().iter().any(|op| op.movable()) && !self.text_editing;
                let mut btn = button::icon(icon_cache_get("select-object-symbolic"))
                    .tooltip(fl!("toolbar-select"));
                if self.viewport.move_mode {
                    btn = btn.class(tool_toggle_class(self.viewport.move_mode));
                }
                let el = has_movable.then(|| {
                    let btn: Element<'_, ViewerMessage> = btn
                        .on_press(ViewerMessage::Edit(EditMessage::ToggleMoveMode))
                        .into();
                    ToolbarItem::new(btn)
                });
                el
            })
            .center(
                ToolbarItem::new(if matches!(self.annotate_tool, AnnotateTool::Text) {
                    self.build_text_format_dropdown()
                } else {
                    self.build_stroke_selector()
                })
                .width_hint(140.0),
            );

        let accent: Color = cosmic::theme::active().cosmic().accent_color().into();
        let swatch_size = 16.0;

        let colors = AnnotateColor::presets();
        for color in &colors {
            let c = *color;
            let is_selected = c == self.annotate_color;
            toolbar = toolbar.center(ToolbarItem::new(
                button::custom(
                    container(Space::new().width(swatch_size).height(swatch_size)).class(
                        cosmic::theme::Container::custom(move |_theme| container::Style {
                            background: Some(Background::Color(c.0)),
                            border: Border {
                                radius: (swatch_size / 2.0).into(),
                                width: if is_selected { 2.0 } else { 0.0 },
                                color: accent,
                            },
                            ..Default::default()
                        }),
                    ),
                )
                .padding(2)
                .class(cosmic::theme::Button::Icon)
                .on_press(ViewerMessage::Edit(EditMessage::AnnotateColor(c))),
            ));
        }

        // Custom color picker button
        let custom_color = self
            .color_picker
            .get_applied_color()
            .unwrap_or(Color::BLACK);
        let is_custom_selected = !colors.contains(&self.annotate_color);

        let inner: Element<'_, ViewerMessage> = if is_custom_selected {
            let filled = container(Space::new().width(swatch_size).height(swatch_size)).class(
                theme::Container::custom(move |_theme| container::Style {
                    background: Some(Background::Color(custom_color)),
                    border: Border {
                        radius: (swatch_size / 2.0).into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            );

            let dashed_accent = DashedBorder::circle(accent, 2.0).dash_pattern(3.0, 3.0);

            let border_layer = canvas(dashed_accent)
                .width(Length::Fixed(swatch_size))
                .height(Length::Fixed(swatch_size));

            stack!()
                .push(filled)
                .push(border_layer)
                .width(Length::Fixed(swatch_size))
                .height(Length::Fixed(swatch_size))
                .into()
        } else {
            let neutral: Color = theme::active().cosmic().palette.neutral_5.into();
            let dashed = DashedBorder::circle(neutral, 2.0).dash_pattern(3.0, 3.0);

            let canvas_bg = canvas(dashed)
                .width(Length::Fixed(swatch_size))
                .height(Length::Fixed(swatch_size));

            let add_icon = container(icon::from_name("list-add-symbolic").size(14).icon())
                .width(Length::Fill)
                .height(Length::Fill)
                .center_x(Length::Fill)
                .center_y(Length::Fill);

            stack!()
                .push(canvas_bg)
                .push(add_icon)
                .width(Length::Fixed(swatch_size))
                .height(Length::Fixed(swatch_size))
                .into()
        };

        let color_picker_btn = button::custom(inner)
            .padding(2)
            .class(theme::Button::Icon)
            .on_press(ViewerMessage::Edit(EditMessage::ColorPicker(
                ToggleColorPicker,
            )));

        let mut color_picker_popover = popover(color_picker_btn);
        if self.color_picker.get_is_active() {
            let picker = self
                .color_picker
                .builder(|u| ViewerMessage::Edit(EditMessage::ColorPicker(u)))
                .reset_label(fl!("color-reset"))
                .build(fl!("color-recent"), fl!("menu-copy"), fl!("color-copied"));

            let popup = container(picker)
                .padding([theme::active().cosmic().spacing.space_s, 0, 0, 0])
                .max_width(260.0)
                .style(|theme| {
                    let cosmic = theme.cosmic();
                    // force opaque because it is not a real popup.
                    let component = &cosmic.background(false).component;
                    container::Style {
                        background: Some(Background::Color(component.base.into())),
                        border: Border {
                            radius: cosmic.radius_s().map(|x| x + 1.0).into(),
                            width: 1.0,
                            color: component.divider.into(),
                        },
                        ..Default::default()
                    }
                });

            let popup = widget::mouse_area(popup).on_press(ViewerMessage::Cancelled);

            color_picker_popover = color_picker_popover
                .popup(popup)
                .position(popover::Position::Top)
                .on_close(ViewerMessage::Edit(EditMessage::ColorPicker(
                    ToggleColorPicker,
                )));
        }

        toolbar = toolbar.center(ToolbarItem::new(color_picker_popover));

        toolbar = toolbar.end(
            ToolbarItem::new(
                button::standard(fl!("toolbar-cancel"))
                    .on_press(ViewerMessage::Edit(EditMessage::AnnotateCancel)),
            )
            .width_hint(96.0),
        );

        toolbar = toolbar.end(ToolbarItem::new(
            button::icon(icon_cache_get("object-select-symbolic"))
                .tooltip(fl!("toolbar-apply"))
                .on_press(ViewerMessage::Edit(EditMessage::AnnotateApply)),
        ));

        toolbar.view()
    }

    fn flatten_image(&self) -> Option<DynamicImage> {
        if let Some(working) = self.viewport.working_image() {
            // `working_image` already has the geometric ops (rotate/crop) baked in by
            // `rebuild_working_image`/`CropApply`. Re-applying them here would double-crop or
            // double-rotate the saved file, so flatten only the overlay (annotation) ops on top.
            let mut image = working.clone();
            for op in self.viewport.operations() {
                if op.as_any().is::<RotateOperation>() || op.as_any().is::<CropOperation>() {
                    continue;
                }
                op.apply(&mut image);
            }
            Some(image)
        } else {
            // No baked working image (nothing edited): flatten every op onto the cached original.
            let current = self.nav.current()?;
            // The operations draw on a copy: the cached original stays as it was loaded.
            let mut image = DynamicImage::clone(&self.cache.get_full(current)?.image);
            for op in self.viewport.operations() {
                op.apply(&mut image);
            }
            Some(image)
        }
    }

    fn build_shape_selector(&self) -> Element<'_, ViewerMessage> {
        let current_icon = self.selected_shape.icon_name();

        let mut trigger = button::custom(
            Row::new()
                .push(icon(icon_cache_get(current_icon)).size(16))
                .push(icon::from_name("pan-down-symbolic").size(12).icon())
                .align_y(Alignment::Center)
                .spacing(2),
        )
        .selected(self.annotate_tool.icon_name() == current_icon)
        .class(tool_toggle_class(
            self.annotate_tool.icon_name() == current_icon,
        ));

        if self.shape_popup {
            trigger = trigger.on_press(ViewerMessage::Edit(EditMessage::ShapePopupClose));
        } else {
            trigger = trigger.on_press(ViewerMessage::Edit(EditMessage::ShapePopupOpen));
        }

        let mut pop = popover(trigger);

        if self.shape_popup {
            let make_btn = |tool: AnnotateTool, label: String| -> Element<'_, ViewerMessage> {
                let is_selected = tool == self.selected_shape;
                let item = Row::new()
                    .push(icon(icon_cache_get(tool.icon_name())).size(16))
                    .push(text::body(label))
                    .push(if is_selected {
                        Element::from(icon::from_name("object-select-symbolic").size(16).icon())
                    } else {
                        Element::from(horizontal().width(16))
                    })
                    .align_y(Alignment::Center)
                    .spacing(8)
                    .width(Length::Shrink);

                button::custom(item)
                    .class(Button::Icon)
                    .width(Length::Shrink)
                    .on_press(ViewerMessage::Edit(EditMessage::AnnotateTool(tool)))
                    .into()
            };

            let list = Column::new()
                .push(make_btn(AnnotateTool::Rectangle, fl!("shapes-rectangle")))
                .push(make_btn(AnnotateTool::Block, fl!("shapes-block")))
                .push(make_btn(AnnotateTool::Ellipse, fl!("shapes-ellipse")))
                .push(make_btn(AnnotateTool::Arrow, fl!("shapes-arrow")))
                .push(make_btn(AnnotateTool::Line, fl!("shapes-line")))
                .push(make_btn(AnnotateTool::Star, fl!("shapes-star")))
                .push(make_btn(AnnotateTool::Polygon, fl!("shapes-polygon")))
                .spacing(2);

            let popup = container(list).padding(8).style(|theme| {
                let cosmic = theme.cosmic();
                // force opaque because it is not a real popup.
                let component = &cosmic.background(false).component;
                container::Style {
                    icon_color: None,
                    text_color: None,
                    background: Some(Background::Color(component.base.into())),
                    border: Border {
                        radius: cosmic.radius_s().map(|x| x + 1.0).into(),
                        width: 1.0,
                        color: component.divider.into(),
                    },
                    ..Default::default()
                }
            });

            pop = pop
                .popup(popup)
                .position(popover::Position::Top)
                .on_close(ViewerMessage::Edit(EditMessage::ShapePopupClose));
        }

        pop.into()
    }

    fn sync_text_format_models(&mut self) {
        let style_entities: Vec<_> = self.text_style_model.iter().collect();

        for entity in style_entities {
            let active = match self.text_style_model.data::<TextStyle>(entity).copied() {
                Some(TextStyle::Bold) => self.text_bold,
                Some(TextStyle::Italic) => self.text_italic,
                Some(TextStyle::Underline) => self.text_underline,
                None => continue,
            };

            if active && !self.text_style_model.is_active(entity) {
                self.text_style_model.activate(entity);
            } else if !active && self.text_style_model.is_active(entity) {
                self.text_style_model.deactivate(entity);
            }
        }

        let align_entity = self.text_align_model.iter().find(|&entity| {
            self.text_align_model.data::<Horizontal>(entity).copied() == Some(self.text_alignment)
        });

        if let Some(entity) = align_entity {
            self.text_align_model.activate(entity);
        }
    }

    // reason: single declarative widget tree; splitting would fragment the layout.
    #[allow(clippy::too_many_lines)]
    fn build_text_format_dropdown(&self) -> Element<'_, ViewerMessage> {
        let trigger = button::custom(
            Row::new()
                .push(text("Aa"))
                .push(icon_cache_get("pan-down-symbolic").icon().size(12))
                .align_y(Alignment::Center)
                .spacing(2)
                .height(Length::Fixed(16.0)),
        )
        .class(theme::Button::Icon)
        .on_press(ViewerMessage::Edit(EditMessage::ToggleTextFormatMenu));

        let mut format_popover = popover(trigger);

        if self.show_text_format_menu {
            // presets are exact integer-valued floats; match the active size to one.
            let font_size_selected = FONT_SIZE_PRESETS_PT
                .iter()
                .position(|pt| (pt_to_px(*pt) - self.text_font_size).abs() < 0.01);

            let font_row = Row::new()
                .push(dropdown(&self.font_families, self.text_font_index, |idx| {
                    ViewerMessage::Edit(EditMessage::TextFontFamily(idx))
                }))
                .push(dropdown(&FONT_SIZE_LABELS, font_size_selected, |idx| {
                    ViewerMessage::Edit(EditMessage::TextFontSize(idx))
                }))
                .align_y(Alignment::Center)
                .spacing(4);

            let style_row = segmented_control::horizontal(&self.text_style_model)
                .on_activate(|entity| ViewerMessage::Edit(EditMessage::TextStyleActivated(entity)));
            let align_row = segmented_control::horizontal(&self.text_align_model)
                .on_activate(|entity| ViewerMessage::Edit(EditMessage::TextAlignActivated(entity)));

            let popup = container(
                Column::new()
                    .push(font_row)
                    .push(style_row)
                    .push(align_row)
                    .spacing(6)
                    .padding(8),
            )
            .style(Self::popup_style)
            .width(Length::Shrink);

            let popup = mouse_area(popup).on_press(ViewerMessage::Cancelled);

            format_popover = format_popover
                .popup(popup)
                .position(popover::Position::Top)
                .on_close(ViewerMessage::Edit(EditMessage::ToggleTextFormatMenu));
        }

        format_popover.into()
    }

    fn build_stroke_selector(&self) -> Element<'_, ViewerMessage> {
        let trigger = button::custom(
            Row::new()
                .push(icon_cache_get("stroke-width-symbolic").icon().size(16))
                .push(icon_cache_get("pan-down-symbolic").icon().size(12))
                .align_y(Alignment::Center)
                .spacing(2),
        )
        .class(cosmic::theme::Button::Icon)
        .on_press(ViewerMessage::Edit(EditMessage::StrokePopupToggle));

        let mut pop = popover(trigger);

        if self.stroke_popup {
            let (labels, sizes, current) = if self.annotate_tool == AnnotateTool::Highlighter {
                (
                    &[
                        "2px", "4px", "6px", "8px", "10px", "12px", "16px", "24px", "32px",
                    ][..],
                    &[2_f32, 4., 6., 8., 10., 12., 16., 24., 32.][..],
                    self.annotate_stroke_size,
                )
            } else {
                (
                    &[
                        "2px", "4px", "6px", "8px", "10px", "12px", "16px", "24px", "32px",
                    ][..],
                    &[2_f32, 4., 6., 8., 10., 12., 16., 24., 32.][..],
                    self.annotate_stroke_size,
                )
            };

            let mut list = Column::new().spacing(4);
            for (label, &size) in labels.iter().zip(sizes.iter()) {
                // presets are exact integer-valued floats; match the active size.
                let is_selected = (size - current).abs() < f32::EPSILON;
                let item: Element<'_, ViewerMessage> = Row::new()
                    .push(text::body(*label))
                    .push(if is_selected {
                        Element::from(icon::from_name("object-select-symbolic").size(16).icon())
                    } else {
                        Element::from(Space::new().width(16))
                    })
                    .align_y(Alignment::Center)
                    .width(Length::Fill)
                    .into();

                // `size` is drawn from `sizes`, so a matching preset always exists.
                let idx = sizes
                    .iter()
                    .position(|s| (*s - size).abs() < f32::EPSILON)
                    .expect("iterated size is one of the presets");
                list = list.push(
                    button::custom(item)
                        .width(Length::Fill)
                        .class(cosmic::theme::Button::Icon)
                        .on_press(ViewerMessage::Edit(EditMessage::AnnotateStroke(idx))),
                );
            }

            let popup = container(list)
                .padding(8)
                .width(Length::Fixed(100.0))
                .style(|theme| {
                    let cosmic = theme.cosmic();
                    // force opaque because it is not a real popup.
                    let component = &cosmic.background(false).component;
                    container::Style {
                        icon_color: None,
                        text_color: None,
                        background: Some(Background::Color(component.base.into())),
                        border: Border {
                            radius: cosmic.radius_s().map(|x| x + 1.0).into(),
                            width: 1.0,
                            color: component.divider.into(),
                        },
                        ..Default::default()
                    }
                });

            pop = pop
                .popup(popup)
                .position(popover::Position::Top)
                .on_close(ViewerMessage::Edit(EditMessage::StrokePopupToggle));
        }

        pop.into()
    }
    /// Multiply the current zoom by `factor`, preserving the fit/crop floor, the 500% ceiling,
    /// and the snap-through-100% behavior the wheel zoom relied on.
    fn zoom_by(&mut self, factor: f32) {
        let bounds = self.viewport.last_bounds().get();
        let viewport = Size::new(bounds.width, bounds.height);
        let before = self.viewport.actual_percent(viewport);
        let old_zoom = self.viewport.zoom();
        let floor = if self.viewport.active_tool() == Some(ToolKind::Crop) {
            1.0
        } else {
            0.1
        };
        let new_zoom = (old_zoom * factor).clamp(floor, 5.0);

        if let Some(size) = self.viewport.image_size()
            && let Some(preview) = self.viewport.preview_mut()
        {
            preview.on_zoom_changed(old_zoom, new_zoom, size);
        }

        self.viewport.set_zoom(new_zoom);

        let after = self.viewport.actual_percent(viewport);

        if (before < 99.9 && after > 100.0) || (before > 100.1 && after < 100.0) {
            self.viewport.set_actual_percent(100.0, viewport);
        }

        // Cap zoom in at 500%
        if factor > 1.0 && self.viewport.actual_percent(viewport) > 500.0 {
            self.viewport.set_actual_percent(500.0, viewport);
        }
    }

    fn settings(&self) -> Element<'_, ViewerMessage> {
        let app_theme_selected = match self.config.app_theme {
            AppTheme::Dark => 1,
            AppTheme::Light => 2,
            AppTheme::System => 0,
        };
        let appearance = widget::settings::section().title(fl!("appearance")).add(
            widget::settings::item::builder(fl!("theme")).control(widget::dropdown(
                &self.app_themes,
                Some(app_theme_selected),
                move |index| {
                    ViewerMessage::AppTheme(match index {
                        1 => AppTheme::Dark,
                        2 => AppTheme::Light,
                        _ => AppTheme::System,
                    })
                },
            )),
        );

        widget::settings::view_column(vec![appearance.into()]).into()
    }
}

impl Application for CosmicViewer {
    const APP_ID: &'static str = "fun.wmde.Viewer";

    type Executor = cosmic::executor::Default;
    // WMDE: every path the command line gave; see viewer_core::args.
    type Flags = Vec<PathBuf>;
    type Message = ViewerMessage;

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    #[allow(clippy::too_many_lines)]
    fn init(mut core: Core, flags: Self::Flags) -> (Self, Task<Action<Self::Message>>) {
        let mut tasks = Vec::new();

        let config = viewer_config::config()
            .ok()
            .as_ref()
            .and_then(|h| ViewerConfig::get_entry(h).ok())
            .unwrap_or_default();

        let scroll_id = Id::unique();

        let families = load_font_families();
        let default_family = match cosmic::font::default().family {
            font::Family::Name(name) => name,
            _ => "Sans",
        };
        let font_index = families.iter().position(|&fam| fam == default_family);

        let initial_color = config
            .last_color
            .map(|c| Color::from_rgba(c[0], c[1], c[2], c[3]));
        let app_themes = vec![fl!("match-desktop"), fl!("dark"), fl!("light")];
        // WMDE: the strip shows for an album of several files, for a directory and for nothing
        // at all, as `open_start` decides them.
        let files = given_files(&flags);
        let album_of_files = flags.len() > 1 && files.len() > 1;
        let directory = (flags.len() < 2 || files.is_empty())
            && flags.first().is_some_and(|path| path.is_dir());
        core.nav_bar_set_toggled(
            config.show_navbar && (flags.is_empty() || album_of_files || directory),
        );
        let mut viewer = Self {
            core,
            cur_scroll: None,
            hovered_item: None,
            key_binds: key_binds::init_keybinds(),
            config,
            app_themes,
            nav: NavState::new(),
            nav_bar_model: nav_bar::Model::default(),
            cache: ImageCache::with_defaults(),
            nav_handles: Vec::new(),
            scroll_id,
            viewport: ViewportManager::default(),
            context_page: None,
            context_menu_position: None,
            annotate_tool: AnnotateTool::default(),
            annotate_color: initial_color.map(AnnotateColor).unwrap_or_default(),
            annotate_stroke_size: 2.,
            crop_ratio: CropRatio::Custom,
            crop_ratio_popup: false,
            text_editing: false,
            move_target: None,
            move_start: None,
            font_families: load_font_families(),
            shape_popup: false,
            stroke_popup: false,
            color_picker: cosmic::widget::ColorPickerModel::new(
                "Hex",
                "RGB",
                None,
                initial_color.or(Some(Color::BLACK)),
            )
            .width(Length::Fixed(248.0))
            .height(Length::Fixed(148.0)),
            has_custom_color: false,
            selected_shape: AnnotateTool::Rectangle,
            text_font_family: default_family,
            text_font_index: font_index,
            text_font_size: pt_to_px(24.0),
            text_bold: false,
            text_italic: false,
            text_underline: false,
            text_alignment: Horizontal::Left,
            text_style_model: segmented_button::Model::builder()
                .insert(|btn| {
                    btn.icon(icon_cache_get("format-text-bold-symbolic").icon())
                        .data(TextStyle::Bold)
                })
                .insert(|btn| {
                    btn.icon(icon_cache_get("format-text-italic-symbolic").icon())
                        .data(TextStyle::Italic)
                })
                .insert(|btn| {
                    btn.icon(icon_cache_get("format-text-underline-symbolic").icon())
                        .data(TextStyle::Underline)
                })
                .build(),
            text_align_model: segmented_button::Model::builder()
                .insert(|btn| {
                    btn.icon(icon::from_name("format-justify-left-symbolic").icon())
                        .data(Horizontal::Left)
                        .activate()
                })
                .insert(|btn| {
                    btn.icon(icon::from_name("format-justify-center-symbolic").icon())
                        .data(Horizontal::Center)
                })
                .insert(|btn| {
                    btn.icon(icon::from_name("format-justify-right-symbolic").icon())
                        .data(Horizontal::Right)
                })
                .build(),
            show_text_format_menu: false,
            window_width: Some(0.0),
            is_fullscreen: false,
            saved_nav_toggled: false,
            wallpaper_dialog: None,
            delete_dialog: None,
            unsaved_dialog: None,
            available_outputs: Vec::new(),
            watcher_rescan_pending: false,
            chosen_files: None,
            was_narrow: false,
            toasts: Toasts::new(ViewerMessage::CloseToast),
            load_error: None,
            canvas_path: None,
            details: None,
            details_stamp: 0,
            wallpapers_running: 0,
            quit_waiting: false,
            about: About::default()
                .name(fl!("app-name"))
                .icon(icon::from_name(Self::APP_ID))
                .version(env!("CARGO_PKG_VERSION"))
                .author("WMDE")
                .comments(fl!("app-description"))
                .license("GPL-3.0-only")
                .developers([("System76", "info@system76.com")])
                .links([
                    (fl!("repository"), "https://github.com/Lin-WMDE/wmde-viewer"),
                    (
                        fl!("support"),
                        "https://github.com/Lin-WMDE/wmde-viewer/issues",
                    ),
                ]),
        };

        tasks.push(viewer.open_start(flags));
        let theme = viewer.config.app_theme.theme();

        tasks.push(cosmic::command::set_theme(theme));

        (viewer, Task::batch(tasks))
    }

    // WMDE: the title bar holds only the title and the window buttons, so the menu bar lives on
    // the strip under it, after the nav bar toggle.
    fn header_strip(&self) -> Vec<Element<'_, Self::Message>> {
        let has_image = self.viewport.image().is_some();

        vec![menu_bar(
            &self.core,
            &self.key_binds,
            self.viewport.can_undo(),
            self.viewport.can_redo(),
            has_image,
        )]
    }

    fn context_drawer(&self) -> Option<context_drawer::ContextDrawer<'_, Self::Message>> {
        let page = self.context_page?;
        let content = match page {
            ContextMessage::ImageDetails => self.image_details_page(),
            ContextMessage::About => {
                return Some(context_drawer::about(
                    &self.about,
                    |s| Self::Message::LaunchUrl(s.to_string()),
                    Self::Message::Context(ContextMessage::About),
                ));
            }
            ContextMessage::Settings => {
                let content = self.settings();
                return Some(
                    context_drawer::context_drawer(content, ViewerMessage::Context(page))
                        .title(fl!("settings"))
                        .into(),
                );
            }
        };

        let drawer = context_drawer::context_drawer(content, ViewerMessage::Context(page))
            .title(fl!("menu-image-details"));
        Some(drawer)
    }

    fn nav_model(&self) -> Option<&nav_bar::Model> {
        Some(&self.nav_bar_model)
    }

    // reason: thumbnail size is a small in-range pixel count; exact as f32.
    #[allow(clippy::cast_precision_loss)]
    fn nav_bar(&self) -> Option<Element<'_, Action<Self::Message>>> {
        if !self.core().nav_bar_active() {
            return None;
        }

        let thumbnail_size = self.config.thumbnail_size.pixels();
        let t = theme::active();
        let t = t.cosmic();
        let space_xxs = f32::from(t.spacing.space_xxs);
        let space_s = f32::from(t.spacing.space_s);
        let panel_width = space_xxs.mul_add(2.0, thumbnail_size as f32) + 36.0;

        let active = self.nav.index().unwrap_or(0);

        let items = self
            .nav
            .images()
            .iter()
            .enumerate()
            .map(|(img, p)| {
                let handle = self
                    .nav_handles
                    .get(img)
                    .and_then(std::clone::Clone::clone)
                    .unwrap_or_else(|| Handle::from_rgba(1, 1, vec![0, 0, 0, 0]));

                let mut btn = container(
                    button::image(handle)
                        .selected(img == active)
                        .height(Length::Fill)
                        .width(Length::Fill)
                        .id(Id::new(format!("img-{}-{img}", p.to_string_lossy())))
                        .on_press(Action::App(ViewerMessage::Nav(NavMessage::GridActivate(
                            img,
                        )))),
                )
                .width(thumbnail_size as f32)
                .height(thumbnail_size as f32)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center);

                if self.hovered_item.is_some_and(|h| h == img) {
                    btn = btn.style(|theme| {
                        let cosmic = theme.cosmic();

                        let component = &cosmic.list_button;
                        container::Style {
                            icon_color: Some(component.on.into()),
                            text_color: Some(component.on.into()),
                            background: Some(Background::Color(
                                cosmic.primary(theme.transparent).component.hover.into(),
                            )),
                            border: Border {
                                radius: cosmic.radius_s().map(|x| x + 1.0).into(),
                                width: 0.0,
                                color: Color::TRANSPARENT,
                            },
                            ..Default::default()
                        }
                    })
                }
                let btn = mouse_area(btn)
                    .on_enter(cosmic::Action::App(ViewerMessage::HoverItem(Some(img))))
                    .on_exit(cosmic::Action::App(ViewerMessage::HoverItem(None)))
                    .on_press(Action::App(ViewerMessage::Nav(NavMessage::GridActivate(
                        img,
                    ))));

                sensor(btn)
                    .on_show(move |_| {
                        Action::App(ViewerMessage::Nav(NavMessage::NavThumbnailShow(img)))
                    })
                    .on_hide(Action::App(ViewerMessage::Nav(
                        NavMessage::NavThumbnailHide(img),
                    )))
                    .into()
            })
            .collect::<Vec<Element<'_, Action<ViewerMessage>>>>();

        let nav_grid = container(
            grid(items)
                .spacing(space_s)
                .columns(1)
                .height(Length::Shrink),
        )
        .padding([0., space_xxs, 0., 0.]);

        let scrollable = scrollable(container(nav_grid).padding(space_xxs))
            .on_scroll(|v| Action::App(ViewerMessage::NavScroll(v)))
            .id(self.scroll_id.clone());

        let nav_width = if self.is_narrow() {
            // Narrow mode: overlay the view, filling window width minus a space_xxs gutter.
            self.window_width.map_or(Length::Fill, |w| {
                Length::Fixed((w - space_xxs).max(panel_width))
            })
        } else {
            Length::Fixed(panel_width)
        };

        Some(
            container(scrollable)
                .width(nav_width)
                .height(Length::Fill)
                .class(theme::Container::custom(nav_bar::nav_bar_style))
                .into(),
        )
    }

    // reason: top-level declarative view tree; splitting would fragment the layout.
    #[allow(clippy::too_many_lines)]
    fn view(&self) -> Element<'_, Self::Message> {
        let has_image = self.viewport.image().is_some();
        // WMDE: the picture that did not open says why, in its own place.
        let load_error = self
            .load_error
            .as_ref()
            .filter(|(path, _)| self.nav.current() == Some(path));
        let content: Element<'_, Self::Message> = if has_image {
            self.viewport.element().map(ViewerMessage::Canvas)
        } else if let Some((path, reason)) = load_error {
            let file = path.file_name().map_or_else(
                || fl!("unnamed-file"),
                |name| name.to_string_lossy().into_owned(),
            );
            Column::new()
                .push(Space::new().height(Length::Fill))
                .push(text::heading(fl!("error-open-title", file = file.as_str())).center())
                .push(text::body(failure::sentence(reason)).center())
                .push(Space::new().height(Length::Fill))
                .spacing(cosmic::theme::active().cosmic().spacing.space_xs)
                .align_x(Horizontal::Center)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            Column::new()
                .push(Space::new().height(Length::Fill))
                .push(text(fl!("no-image-selected")).center())
                .push(Space::new().height(Length::Fill))
                .align_x(Horizontal::Center)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        };

        let spacing = cosmic::theme::active().cosmic().spacing;

        let nav_collapsed = !self.core().nav_bar_active();
        let cur_idx = self.nav.index().unwrap_or(0);

        // WMDE: a picture that did not open keeps the buttons that lead past it.
        let image_area: Element<'_, Self::Message> =
            if nav_collapsed && (has_image || load_error.is_some()) {
                let has_prev = cur_idx > 0;
                let has_next = cur_idx + 1 < self.nav.total();

                let nav_btn = |icon_name: &'static str,
                               msg: ViewerMessage,
                               enabled: bool|
                 -> Element<'_, Self::Message> {
                    let mut btn = button::icon(icon::from_name(icon_name).size(24))
                        .class(cosmic::theme::Button::Icon);
                    if enabled {
                        btn = btn.on_press(msg);
                    }
                    container(btn).center_y(Length::Fill).into()
                };

                Row::new()
                    .push(nav_btn(
                        "go-previous-symbolic",
                        ViewerMessage::Nav(NavMessage::GridActivate(cur_idx.saturating_sub(1))),
                        has_prev,
                    ))
                    .push(container(content).width(Length::Fill).height(Length::Fill))
                    .push(nav_btn(
                        "go-next-symbolic",
                        ViewerMessage::Nav(NavMessage::GridActivate(
                            (cur_idx + 1).min(self.nav.total().saturating_sub(1)),
                        )),
                        has_next,
                    ))
                    .align_y(Alignment::Center)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            } else {
                container(content)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            };

        let main = Column::new()
            .push(image_area)
            .push_maybe(has_image.then(|| {
                container(self.build_toolbar())
                    .center_x(Length::Fill)
                    .padding([spacing.space_xxs, 0, spacing.space_xxs, 0])
            }))
            .width(Length::Fill)
            .height(Length::Fill);

        let mut pop = widget::popover(main);
        if let Some(point) = self.context_menu_position
            && self.wallpaper_dialog.is_none()
            && self.delete_dialog.is_none()
        {
            pop = pop
                .popup(Self::build_context_menu_element())
                .position(widget::popover::Position::Point(point))
                .on_close(ViewerMessage::Canvas(CanvasMessage::ContextMenu(None)));
        }

        let view: Element<'_, Self::Message> = pop.into();

        if let Some(ref path) = self.wallpaper_dialog {
            let path = path.clone();
            let spacing = cosmic::theme::active().cosmic().spacing;

            let mut btn_col = Column::new().spacing(spacing.space_s);
            btn_col = btn_col.push(button::standard(fl!("wallpaper-all-displays")).on_press(
                ViewerMessage::SetWallpaperOn(path.clone(), crate::message::WallpaperTarget::All),
            ));
            for output in &self.available_outputs {
                btn_col = btn_col.push(button::standard(output.clone()).on_press(
                    ViewerMessage::SetWallpaperOn(
                        path.clone(),
                        crate::message::WallpaperTarget::Output(output.clone()),
                    ),
                ));
            }

            let dialog: Element<'_, Self::Message> = container(
                container(
                    Column::new()
                        .push(text::title4(fl!("wallpaper-dialog-title")))
                        .push(btn_col)
                        .push(
                            button::text(fl!("wallpaper-cancel"))
                                .on_press(ViewerMessage::CloseWallpaperDialog),
                        )
                        .spacing(spacing.space_s)
                        .align_x(Alignment::Center),
                )
                .padding(spacing.space_m)
                .class(cosmic::theme::Container::Dialog(true)),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into();

            let backdrop = cosmic::widget::mouse_area(
                container(Space::new().width(Length::Fill).height(Length::Fill))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .class(cosmic::theme::Container::Transparent),
            )
            .on_press(ViewerMessage::CloseWallpaperDialog);

            return toaster(&self.toasts, stack![view, backdrop, dialog]);
        }

        toaster(&self.toasts, view)
    }

    fn dialog(&self) -> Option<Element<'_, Self::Message>> {
        if self.unsaved_dialog.is_some() {
            let mut row = widget::row::with_capacity(3).align_y(Alignment::Center);
            row = row.push(widget::text(
                self.nav
                    .current()
                    .and_then(|d| Some(d.file_name()?.to_string_lossy().to_string()))
                    .unwrap_or_default(),
            ));
            row = row.push(widget::space::horizontal());
            row = row.push(
                widget::button::standard(fl!("unsaved-save-as"))
                    .on_press(ViewerMessage::Unsaved(UnsavedChoice::SaveAs)),
            );
            let save_button = widget::button::suggested(fl!("unsaved-save"))
                .on_press(ViewerMessage::Unsaved(UnsavedChoice::Save));
            let discard_button = widget::button::destructive(fl!("unsaved-discard"))
                .on_press(ViewerMessage::Unsaved(UnsavedChoice::Discard));
            let cancel_button = widget::button::text(fl!("unsaved-cancel"))
                .on_press(ViewerMessage::Unsaved(UnsavedChoice::Cancel));
            let dialog = widget::dialog()
                .title(fl!("unsaved-dialog-title"))
                .body(fl!("unsaved-dialog-body"))
                .icon(icon::from_name("dialog-warning-symbolic").size(64))
                .control(row)
                .primary_action(save_button)
                .secondary_action(discard_button)
                .tertiary_action(cancel_button);
            return Some(dialog.into());
        }
        None
    }
    // reason: central message dispatch; one match arm per variant, kept colocated.
    #[allow(clippy::too_many_lines)]
    fn update(&mut self, message: Self::Message) -> Task<Action<Self::Message>> {
        // Helper for updating config values efficiently
        macro_rules! config_set {
            ($name: ident, $value: expr) => {
                match viewer_config::config() {
                    Ok(config_handler) => {
                        if let Err(err) =
                            paste::paste! { self.config.[<set_ $name>](&config_handler, $value) }
                        {
                            tracing::warn!(
                                "failed to save config {:?}: {}",
                                stringify!($name),
                                err
                            );
                        }
                    }
                    Err(err) => {
                        self.config.$name = $value;
                        tracing::warn!(
                            "failed to save config {:?}: no config handler - {err:?}",
                            stringify!($name)
                        );
                    }
                }
            };
        }
        let mut tasks = vec![];

        match message {
            ViewerMessage::AppTheme(app_theme) => {
                config_set!(app_theme, app_theme);
                let theme = self.config.app_theme.theme();

                return cosmic::command::set_theme(theme);
            }
            ViewerMessage::CopyToClipboard => {
                self.context_menu_position = None;
                if let Some(path) = self.nav.current().cloned()
                    && let Some(mime) = path
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .and_then(image_mime_type)
                    && let Ok(data) = std::fs::read(&path)
                {
                    return clipboard::write_data(ClipboardImage {
                        data,
                        mime: mime.to_string(),
                    });
                }
            }
            ViewerMessage::CopyFilePath => {
                self.context_menu_position = None;
                if let Some(path) = self.nav.current() {
                    return cosmic::iced::clipboard::write(path.display().to_string());
                }
            }
            // No-op messages: reserved menu actions with no current behavior.
            ViewerMessage::Cancelled => {}
            ViewerMessage::OpenFileDialog => {
                if self.guard_unsaved(ViewerMessage::OpenFileDialog) {
                    return Task::none();
                }
                return future(async move {
                    // WMDE: every extension the album takes, in both cases: the portal
                    // matches a pattern as written.
                    let all = nav::extensions().fold(
                        FileFilter::new(fl!("filter-all")),
                        |filter, extension| {
                            filter
                                .extension(extension)
                                .extension(extension.to_uppercase())
                        },
                    );
                    let raw = viewer_core::raw::RAW_TYPES.iter().fold(
                        FileFilter::new("RAW"),
                        |filter, (extension, _)| {
                            filter
                                .extension(*extension)
                                .extension(extension.to_uppercase())
                        },
                    );
                    let dialog = file_chooser::open::Dialog::new()
                        .title(fl!("dialog-open-image"))
                        .filter(all)
                        .filter(
                            FileFilter::new("JPEG")
                                .extension("jpg")
                                .extension("jpeg")
                                .extension("jpe")
                                .extension("JPEG")
                                .extension("JPG")
                                .extension("JPE"),
                        )
                        .filter(FileFilter::new("png").extension("png").extension("PNG"))
                        .filter(FileFilter::new("gif").extension("gif").extension("GIF"))
                        .filter(FileFilter::new("webp").extension("webp").extension("WEBP"))
                        .filter(FileFilter::new("bmp").extension("bmp").extension("BMP"))
                        .filter(
                            FileFilter::new("tiff")
                                .extension("tiff")
                                .extension("tif")
                                .extension("TIFF")
                                .extension("TIF"),
                        )
                        .filter(FileFilter::new("avif").extension("avif").extension("AVIF"))
                        .filter(FileFilter::new("ico").extension("ico").extension("ICO"))
                        .filter(FileFilter::new("hdr").extension("hdr").extension("HDR"))
                        .filter(
                            FileFilter::new("SVG")
                                .extension("svg")
                                .extension("SVG")
                                .extension("svgz")
                                .extension("SVGZ"),
                        )
                        .filter(raw);

                    match dialog.open_file().await {
                        Ok(response) => response.url().to_file_path().map_or_else(
                            |()| Action::App(ViewerMessage::Cancelled),
                            |path| Action::App(ViewerMessage::Open(path)),
                        ),
                        Err(file_chooser::Error::Cancelled) => {
                            Action::App(ViewerMessage::Cancelled)
                        }
                        Err(e) => {
                            tracing::error!("File dialog error: {e}");
                            Action::App(ViewerMessage::ShowError(fl!("error-dialog")))
                        }
                    }
                });
            }
            ViewerMessage::OpenFolderDialog => {
                if self.guard_unsaved(ViewerMessage::OpenFolderDialog) {
                    return Task::none();
                }
                return future(async move {
                    let dialog = file_chooser::open::Dialog::new().title(fl!("dialog-open-folder"));

                    match dialog.open_folder().await {
                        Ok(response) => response.url().to_file_path().map_or_else(
                            |()| Action::App(ViewerMessage::Cancelled),
                            |path| Action::App(ViewerMessage::Open(path)),
                        ),
                        Err(file_chooser::Error::Cancelled) => {
                            Action::App(ViewerMessage::Cancelled)
                        }
                        Err(e) => {
                            tracing::error!("Folder dialog error: {e}");
                            Action::App(ViewerMessage::ShowError(fl!("error-dialog")))
                        }
                    }
                });
            }
            ViewerMessage::Open(path) => tasks.push(self.open_path(path)),
            ViewerMessage::OpenRecent(_idx) => {}
            ViewerMessage::OpenContaining => {
                // WMDE: off the thread that draws the window. `open::that` waits for the opener
                // to exit, which is what reports a failure (no handler for folders, a folder
                // gone); `that_detached` would return before it and report nothing. A plain
                // thread, not tokio's blocking pool: the runtime waits for that pool on exit,
                // and a file manager kept in the foreground would keep the viewer alive.
                // WMDE: the current picture's directory, which in an album of files given is
                // not always the album's.
                let dir = self
                    .nav
                    .current()
                    .and_then(|current| current.parent().map(Path::to_path_buf))
                    .or_else(|| self.nav.dir().map(Path::to_path_buf));
                if let Some(dir) = dir {
                    let (sender, receiver) = tokio::sync::oneshot::channel();
                    let opener_dir = dir.clone();
                    std::thread::spawn(move || {
                        _ = sender.send(open::that(opener_dir).map_err(|err| err.to_string()));
                    });
                    tasks.push(future(async move {
                        let failure = match receiver.await {
                            Ok(Ok(())) => return Action::App(ViewerMessage::Cancelled),
                            Ok(Err(err)) => err,
                            Err(err) => err.to_string(),
                        };
                        tracing::error!("cannot open {}: {failure}", dir.display());
                        Action::App(ViewerMessage::ShowError(fl!("error-open-folder")))
                    }));
                }
            }
            ViewerMessage::Save => match self.save_in_place() {
                Ok(task) | Err(task) => tasks.push(task),
            },
            ViewerMessage::SaveAs => {
                let current = self.nav.current();
                // WMDE: a file the viewer cannot write, a camera RAW or a drawing among them, is
                // offered as a PNG; under its own name the save could only fail.
                let writable = |path: &Path| {
                    image::ImageFormat::from_path(path).is_ok_and(|format| format.writing_enabled())
                };
                let suggested = current
                    .and_then(|path| {
                        if writable(path) {
                            path.file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                        } else {
                            path.file_stem()
                                .map(|stem| format!("{}.png", stem.to_string_lossy()))
                        }
                    })
                    .unwrap_or_else(|| "image.png".to_string());
                let directory =
                    current.and_then(|path| path.parent().map(std::path::Path::to_path_buf));

                return future(async move {
                    let mut dialog = file_chooser::save::Dialog::new()
                        .title(fl!("dialog-save-image-as"))
                        .file_name(suggested);
                    if let Some(directory) = directory {
                        dialog = dialog.directory(directory);
                    }

                    match dialog.save_file().await {
                        // WMDE: no URL is how the rfd backend reports both a cancel and a
                        // portal failure. It was an `expect`, and the release profile aborts on
                        // panic, so cancelling Save As closed the viewer and lost the edits.
                        Ok(response) => response
                            .url()
                            .and_then(|url| url.to_file_path().ok())
                            .map_or(Action::App(ViewerMessage::Cancelled), |path| {
                                Action::App(ViewerMessage::SavedAs(path))
                            }),
                        Err(file_chooser::Error::Cancelled) => {
                            Action::App(ViewerMessage::Cancelled)
                        }
                        Err(e) => {
                            tracing::error!("Save dialog error: {e}");
                            Action::App(ViewerMessage::ShowError(fl!("error-dialog")))
                        }
                    }
                });
            }
            ViewerMessage::SavedAs(path) => {
                // WMDE: a failed save keeps the edits and cancels whatever was waiting on it;
                // carrying on would drop the edits the save was meant to keep.
                if let Err(complaint) = self.write_edits(&path, false) {
                    self.unsaved_dialog = None;
                    return complaint;
                }
                // WMDE: see save_in_place. A picture saved over itself is read again here, as
                // there, rather than left to the watcher, which is not the only one to say so.
                let reload = if self.nav.current() == Some(&path) {
                    self.details_stamp = self.details_stamp.wrapping_add(1);
                    self.cache.remove_full(&path);
                    self.cache.remove_thumbnail(&path);
                    self.load_full_image(path)
                } else {
                    Task::none()
                };

                self.viewport.cancel_tool();
                self.viewport.revert_all();
                return Task::batch([reload, self.resolve_pending()]);
            }
            ViewerMessage::Quit => {
                // WMDE: a wallpaper still being set is finished first: the thread would die
                // with the process, possibly between two of the settings it writes. A second
                // quit, or 20 s, ends the wait.
                if self.wallpapers_running > 0 && !self.quit_waiting {
                    self.quit_waiting = true;
                    return future(async {
                        tokio::time::sleep(Duration::from_secs(20)).await;
                        Action::App(ViewerMessage::Quit)
                    });
                }
                return iced::exit();
            }
            ViewerMessage::CloseRequested => {
                if self.has_unsaved_edits() {
                    self.unsaved_dialog = Some(PendingAction::Quit);
                } else {
                    return self.update(ViewerMessage::Quit);
                }
            }
            ViewerMessage::Unsaved(choice) => match choice {
                UnsavedChoice::Save => {
                    return match self.save_in_place() {
                        Ok(reload) => Task::batch([reload, self.resolve_pending()]),
                        // WMDE: see SavedAs - the pending action is dropped with the edits kept.
                        Err(complaint) => {
                            self.unsaved_dialog = None;
                            complaint
                        }
                    };
                }
                UnsavedChoice::SaveAs => return self.update(ViewerMessage::SaveAs),
                UnsavedChoice::Discard => {
                    self.viewport.cancel_tool();
                    self.viewport.revert_all();
                    self.text_editing = false;
                    return self.resolve_pending();
                }
                UnsavedChoice::Cancel => {
                    self.unsaved_dialog = None;
                }
            },
            ViewerMessage::TextPaste(text) => {
                if self.text_editing && !text.is_empty() {
                    if let Some(preview) = self.viewport.preview_mut()
                        && let Some(t) = preview.as_any_mut().downcast_mut::<TextPreview>()
                    {
                        t.insert_with_attrs(&text);
                    }
                    self.viewport.rebuild_display();
                }
            }
            ViewerMessage::WatcherEvent(evt) => {
                // WMDE: the details drawer reads a changed file again.
                if let WatcherEvent::Created(path) | WatcherEvent::Modified(path) = &evt
                    && self.nav.current() == Some(path)
                {
                    self.details_stamp = self.details_stamp.wrapping_add(1);
                }
                match &evt {
                    WatcherEvent::Modified(path) if path.exists() => {
                        // WMDE: a load already under way read the file before this change;
                        // the rescan after it starts a fresh one.
                        self.cache.clear_pending(path);
                        self.cache.remove_full(path);
                        self.cache.remove_thumbnail(path);
                    }
                    WatcherEvent::Modified(path) | WatcherEvent::Removed(path) => {
                        self.cache.clear_pending(path);
                        self.cache.clear_pending_thumbnail(path);
                        self.cache.remove_full(path);
                        self.cache.remove_thumbnail(path);
                        if self.nav.current() == Some(path) {
                            self.nav.deselect();
                        }
                    }
                    WatcherEvent::Created(_) => {}
                    WatcherEvent::Error(err) => {
                        tracing::warn!("watcher error: {err}");
                    }
                }

                if !matches!(evt, WatcherEvent::Error(_)) && !self.watcher_rescan_pending {
                    self.watcher_rescan_pending = true;
                    tasks.push(future(async {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        Action::App(ViewerMessage::WatcherRescan)
                    }));
                }
            }
            ViewerMessage::WatcherRescan => {
                self.watcher_rescan_pending = false;
                tasks.push(self.reload_image_list());
            }
            ViewerMessage::SetWallpaper => {
                self.context_menu_position = None;
                if let Some(path) = self.nav.current().cloned() {
                    #[cfg(target_os = "linux")]
                    {
                        if is_cosmic_desktop() {
                            match self.config.wallpaper_behavior {
                                viewer_config::WallpaperBehavior::Ask => {
                                    self.available_outputs = get_cosmic_outputs();
                                    if self.available_outputs.len() <= 1 {
                                        return self.wallpaper_task(path, None);
                                    }
                                    self.wallpaper_dialog = Some(path);
                                }
                                viewer_config::WallpaperBehavior::AllDisplays => {
                                    return self.wallpaper_task(path, None);
                                }
                                viewer_config::WallpaperBehavior::CurrentDisplay => {
                                    let outputs = get_cosmic_outputs();
                                    return self.wallpaper_task(path, outputs.first().cloned());
                                }
                            }
                        } else {
                            // Non-WMDE Linux: set on all outputs
                            return self.wallpaper_task(path, None);
                        }
                    }
                    #[cfg(not(target_os = "linux"))]
                    {
                        let result = set_wallpaper_portable(&path);
                        return self.update(ViewerMessage::WallpaperResult(result));
                    }
                }
            }
            ViewerMessage::SetWallpaperOn(path, target) => {
                self.wallpaper_dialog = None;
                let output = match target {
                    crate::message::WallpaperTarget::All => None,
                    crate::message::WallpaperTarget::Output(name) => Some(name),
                };
                return self.wallpaper_task(path, output);
            }
            ViewerMessage::CloseWallpaperDialog => {
                self.wallpaper_dialog = None;
            }
            ViewerMessage::WallpaperResult(result) => {
                self.wallpapers_running = self.wallpapers_running.saturating_sub(1);
                if let Err((reason, detail)) = result {
                    tasks.push(self.complain(
                        "set the wallpaper",
                        &detail,
                        fl!("error-wallpaper", error = reason),
                    ));
                }
                if self.quit_waiting && self.wallpapers_running == 0 {
                    return iced::exit();
                }
            }
            ViewerMessage::MoveToTrash => {
                self.context_menu_position = None;
                // WMDE: unsaved edits are settled before the file goes, not after: Cancel keeps
                // the file, and Discard replays the deletion against the album as it is then.
                // WMDE: a path that is not there has nothing to trash; it stays, with its reason.
                let present = self.nav.current().is_some_and(|path| path.exists());
                if present && self.guard_unsaved(ViewerMessage::MoveToTrash) {
                    return Task::none();
                }
                if let Some(path) = self.nav.current().filter(|_| present).cloned() {
                    // WMDE: the picture leaves the album at once and the one after it takes its
                    // place, the one before only when the last is deleted, as in wmde-photos:
                    // throwing out the bad shots of a series moves forward, not back to one
                    // already seen. Waiting for the rescan after the trash left the file in the
                    // album for 200 ms, and a second Delete in that window picked it again.
                    self.nav.forget_current();
                    let next_idx = self.nav.index();
                    self.rebuild_nav_handles();
                    if let Some(idx) = next_idx {
                        tasks.push(self.update(ViewerMessage::Nav(NavMessage::GridActivate(idx))));
                    } else {
                        self.clear_canvas();
                    }

                    // Trash the file
                    tasks.push(future(async move {
                        let result = trash::delete(&path)
                            .map_err(|e| (failure::trash_reason(&e), format!("{e:?}")));
                        Action::App(ViewerMessage::TrashResult(path, result))
                    }));
                }
            }
            ViewerMessage::UndoTrash(toast_id, recently_trashed) => {
                self.toasts.remove(toast_id);

                tasks.push(future(async move {
                    // Rescan trash to find matching TrashItem entries
                    let mut items_to_restore = Vec::new();

                    // WMDE: an unreadable trash is reported instead of restoring nothing.
                    let trash_items = match trash::os_limited::list() {
                        Ok(items) => items,
                        Err(e) => {
                            tracing::error!("cannot list the trash: {e:?}");
                            return Action::App(ViewerMessage::ShowError(fl!(
                                "error-restore",
                                error = failure::trash_reason(&e)
                            )));
                        }
                    };
                    for path in &*recently_trashed {
                        // WMDE: the trash records the canonical parent; a path given relative
                        // or through a linked folder is compared the same way.
                        let given_parent = path
                            .parent()
                            .filter(|dir| !dir.as_os_str().is_empty())
                            .unwrap_or_else(|| Path::new("."));
                        let target_parent = given_parent
                            .canonicalize()
                            .unwrap_or_else(|_| given_parent.to_path_buf());
                        // WMDE: compared as OS strings: a lossy conversion never matches a
                        // name that is not UTF-8.
                        let target_name = path.file_name().unwrap_or_default();

                        for item in &trash_items {
                            if item.original_parent == target_parent
                                && item.name.as_os_str() == target_name
                            {
                                items_to_restore.push(item.clone());
                                break;
                            }
                        }
                    }
                    Action::App(ViewerMessage::UndoTrashStart(items_to_restore))
                }));
            }
            ViewerMessage::UndoTrashStart(items) => {
                if items.is_empty() {
                    // WMDE: the file left the trash some other way; Undo says so.
                    tasks.push(self.complain(
                        "restore from the trash",
                        "no matching trash entry",
                        fl!("error-restore", error = fl!("error-reason-not-in-trash")),
                    ));
                } else {
                    let restore = future(async move {
                        if let Err(e) = trash::os_limited::restore_all(items) {
                            tracing::error!("cannot restore from the trash: {e:?}");
                            return Action::App(ViewerMessage::ShowError(fl!(
                                "error-restore",
                                error = failure::trash_reason(&e)
                            )));
                        }

                        // Trigger a directory rescan
                        Action::App(ViewerMessage::Cancelled)
                    });

                    // Rescan the directory to pickup the restored image
                    tasks.push(restore.chain(self.reload_image_list()));
                }
            }
            ViewerMessage::TrashResult(path, result) => {
                let file_name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map_or_else(|| fl!("unnamed-file"), str::to_string);
                match result {
                    Ok(()) => {
                        let paths: Arc<[PathBuf]> = Arc::from(vec![path]);
                        let label = fl!("moved-to-trash", file = file_name.as_str());
                        // Auto-dismiss (the Task `push` returns) needs libcosmic's "tokio" feature.
                        let toast = toaster::Toast::new(label)
                            .action(fl!("toast-undo"), move |id| {
                                ViewerMessage::UndoTrash(id, paths.clone())
                            })
                            .duration(toaster::Duration::Long);
                        tasks.push(self.toasts.push(toast).map(Action::App));
                    }
                    Err((reason, detail)) => {
                        // WMDE: the file stayed; a rescan puts it back into the album.
                        tasks.push(self.reload_image_list());
                        tasks.push(self.complain(
                            &format!("move {} to the trash", path.display()),
                            &detail,
                            fl!("error-trash", file = file_name.as_str(), error = reason),
                        ));
                    }
                }
            }
            ViewerMessage::ShowError(message) => tasks.push(self.show_error(message)),
            ViewerMessage::DetailsRead(path, stamp, details) => {
                // A reading of a picture since left, or of the file before its latest change,
                // is dropped.
                if self.context_page == Some(ContextMessage::ImageDetails)
                    && stamp == self.details_stamp
                    && self.nav.current() == Some(&path)
                {
                    self.details = Some((path, details));
                }
            }
            ViewerMessage::CloseToast(toast_id) => {
                self.toasts.remove(toast_id);
            }
            ViewerMessage::KeyPressed(key, modifiers, text) => {
                if self.context_menu_position.is_some() && matches!(key, Key::Named(Named::Escape))
                {
                    self.context_menu_position = None;
                    return Task::none();
                }

                // Enter applies crop, Escape cancels
                if matches!(key, Key::Named(Named::Enter))
                    && self.viewport.active_tool() == Some(ToolKind::Crop)
                {
                    return self.update(ViewerMessage::Edit(EditMessage::CropApply));
                }

                if matches!(key, Key::Named(Named::Escape))
                    && self.viewport.active_tool() == Some(ToolKind::Crop)
                {
                    return self.update(ViewerMessage::Edit(EditMessage::CropCancel));
                }

                if self.viewport.move_mode && matches!(key, Key::Named(Named::Escape)) {
                    self.viewport.move_mode = false;
                    self.move_target = None;
                    self.viewport.select_target = None;
                    self.move_start = None;
                    return Task::none();
                }

                // Picker open: Escape dismisses just the picker, not markup mode.
                if self.color_picker.get_is_active() && matches!(key, Key::Named(Named::Escape)) {
                    return self.update(ViewerMessage::Edit(EditMessage::ColorPicker(
                        ToggleColorPicker,
                    )));
                }
                // WMDE: the rest of the keys belong to the picker's hex field while it is open. Its
                // text input lets ArrowUp and ArrowDown through uncaptured, and they turned the
                // picture or moved the annotation's caret.
                if self.color_picker.get_is_active() {
                    return Task::none();
                }

                if matches!(key, Key::Named(Named::Escape))
                    && self.viewport.active_tool() == Some(ToolKind::Annotate)
                    && !self.text_editing
                {
                    self.viewport.apply_tool();
                    self.viewport.set_active_tool(None);
                    self.viewport.set_preview(None);
                    self.viewport.rebuild_display();
                    return Task::none();
                }

                if self.is_fullscreen && matches!(key, Key::Named(Named::Escape)) {
                    return self.update(ViewerMessage::Canvas(CanvasMessage::Fullscreen));
                }

                if matches!(key, Key::Named(Named::Delete)) && self.viewport.active_tool().is_none()
                {
                    return self.update(ViewerMessage::MoveToTrash);
                }

                let is_text_editing = self.text_editing
                    && self
                        .viewport
                        .preview_mut()
                        .and_then(|preview| preview.as_any_mut().downcast_mut::<TextPreview>())
                        .is_some();

                let preview = self
                    .viewport
                    .preview_mut()
                    .and_then(|preview| preview.as_any_mut().downcast_mut::<TextPreview>());

                if is_text_editing {
                    let shift = modifiers.shift();

                    if modifiers.control() {
                        if let Key::Character(c) = &key {
                            match c.as_str() {
                                "b" => {
                                    self.text_bold = !self.text_bold;
                                    if let Some(preview) = self.viewport.preview_mut()
                                        && let Some(t) =
                                            preview.as_any_mut().downcast_mut::<TextPreview>()
                                    {
                                        if t.has_selection() {
                                            let bold = self.text_bold;
                                            t.apply_attr_to_selection(|a| {
                                                a.weight(if bold {
                                                    cosmic_text::Weight::BOLD
                                                } else {
                                                    cosmic_text::Weight::NORMAL
                                                })
                                            });
                                        }
                                        t.bold = self.text_bold;
                                    }
                                    self.sync_text_format_models();
                                    return Task::none();
                                }
                                "i" => {
                                    self.text_italic = !self.text_italic;
                                    if let Some(preview) = self.viewport.preview_mut()
                                        && let Some(t) =
                                            preview.as_any_mut().downcast_mut::<TextPreview>()
                                    {
                                        if t.has_selection() {
                                            let italic = self.text_italic;
                                            t.apply_attr_to_selection(|a| {
                                                a.style(if italic {
                                                    cosmic_text::Style::Italic
                                                } else {
                                                    cosmic_text::Style::Normal
                                                })
                                            });
                                        }
                                        t.italic = self.text_italic;
                                    }
                                    self.sync_text_format_models();
                                    return Task::none();
                                }
                                "u" => {
                                    self.text_underline = !self.text_underline;
                                    if let Some(preview) = self.viewport.preview_mut()
                                        && let Some(t) =
                                            preview.as_any_mut().downcast_mut::<TextPreview>()
                                    {
                                        if t.has_selection() {
                                            let underline = self.text_underline;
                                            t.apply_attr_to_selection(|a| {
                                                a.metadata(usize::from(underline))
                                            });
                                        }
                                        t.underline = self.text_underline;
                                    }
                                    self.sync_text_format_models();
                                    return Task::none();
                                }
                                "a" => {
                                    if let Some(preview) = self.viewport.preview_mut()
                                        && let Some(t) =
                                            preview.as_any_mut().downcast_mut::<TextPreview>()
                                    {
                                        t.select_all();
                                    }
                                    return Task::none();
                                }
                                "c" => {
                                    if let Some(preview) = self.viewport.preview_ref()
                                        && let Some(t) =
                                            preview.as_any().downcast_ref::<TextPreview>()
                                        && let Some(sel) = t.copy_selection()
                                    {
                                        return cosmic::iced::clipboard::write(sel);
                                    }
                                    return Task::none();
                                }
                                "v" => {
                                    return cosmic::iced::clipboard::read().map(|opt| {
                                        cosmic::Action::App(ViewerMessage::TextPaste(
                                            opt.unwrap_or_default(),
                                        ))
                                    });
                                }
                                "x" => {
                                    if let Some(preview) = self.viewport.preview_ref()
                                        && let Some(t) =
                                            preview.as_any().downcast_ref::<TextPreview>()
                                        && let Some(sel) = t.copy_selection()
                                    {
                                        let task = cosmic::iced::clipboard::write(sel);
                                        if let Some(preview) = self.viewport.preview_mut()
                                            && let Some(t) =
                                                preview.as_any_mut().downcast_mut::<TextPreview>()
                                        {
                                            t.delete_selection();
                                        }
                                        return task;
                                    }
                                    return Task::none();
                                }
                                _ => {}
                            }
                        }
                        return Task::none();
                    }

                    match &key {
                        Key::Named(Named::Enter) if shift => {
                            if let Some(preview) = preview {
                                preview.editor_action(cosmic_text::Action::Enter);
                            }
                        }
                        Key::Named(Named::Enter | Named::Escape) => {
                            if self.color_picker.get_is_active() {
                                _ = self.color_picker.update::<ViewerMessage>(
                                    color_picker::ColorPickerUpdate::ToggleColorPicker,
                                );
                            }
                            self.viewport.apply_tool();
                            self.text_editing = false;
                            self.text_bold = false;
                            self.text_italic = false;
                            self.text_underline = false;
                            self.viewport.set_active_tool(Some(ToolKind::Annotate));
                            self.viewport.set_preview(Some(Box::new(TextPreview::new(
                                self.annotate_color.0,
                                self.text_font_size,
                                self.text_font_family,
                                false,
                                false,
                                false,
                                self.text_alignment,
                                fl!("type-here"),
                            ))));
                            self.sync_text_format_models();
                        }
                        Key::Named(Named::Backspace) => {
                            if let Some(preview) = preview {
                                preview.editor_action(cosmic_text::Action::Backspace);
                            }
                        }
                        Key::Named(Named::Delete) => {
                            if let Some(preview) = preview {
                                preview.editor_action(cosmic_text::Action::Delete);
                            }
                        }
                        Key::Named(Named::ArrowLeft) => {
                            if let Some(preview) = preview {
                                preview.motion_with_shift(cosmic_text::Motion::Left, shift);
                            }
                        }
                        Key::Named(Named::ArrowRight) => {
                            if let Some(preview) = preview {
                                preview.motion_with_shift(cosmic_text::Motion::Right, shift);
                            }
                        }
                        Key::Named(Named::ArrowUp) => {
                            if let Some(preview) = preview {
                                preview.motion_with_shift(cosmic_text::Motion::Up, shift);
                            }
                        }
                        Key::Named(Named::ArrowDown) => {
                            if let Some(preview) = preview {
                                preview.motion_with_shift(cosmic_text::Motion::Down, shift);
                            }
                        }
                        Key::Named(Named::Home) => {
                            if let Some(preview) = preview {
                                preview.motion_with_shift(cosmic_text::Motion::Home, shift);
                            }
                        }
                        Key::Named(Named::End) => {
                            if let Some(preview) = preview {
                                preview.motion_with_shift(cosmic_text::Motion::End, shift);
                            }
                        }
                        Key::Character(c) if c.as_str() == " " => {
                            if let Some(preview) = preview {
                                preview.insert_with_attrs(" ");
                            }
                        }
                        _ => {
                            if let Some(txt) = &text
                                && let Some(preview) = preview
                            {
                                for ch in txt.chars() {
                                    let mut buf = [0u8; 4];
                                    preview.insert_with_attrs(ch.encode_utf8(&mut buf));
                                }
                            }
                        }
                    }

                    let is_modifier = matches!(
                        key,
                        Key::Named(
                            Named::Shift
                                | Named::Control
                                | Named::Alt
                                | Named::Super
                                | Named::CapsLock
                        )
                    );
                    if !is_modifier
                        && let Some(preview) = self.viewport.preview_mut()
                        && let Some(t) = preview.as_any_mut().downcast_mut::<TextPreview>()
                    {
                        if !t.sync_format_over_selection() {
                            t.sync_format_at_cursor();
                        }
                        self.text_bold = t.bold;
                        self.text_italic = t.italic;
                        self.text_underline = t.underline;
                        self.text_alignment = t.alignment;
                        self.text_font_size = t.font_size;
                        self.text_font_family = t.font_family;
                        self.text_font_index =
                            self.font_families.iter().position(|&f| f == t.font_family);
                        self.annotate_color = AnnotateColor(t.color);
                    }
                    self.sync_text_format_models();
                } else if modifiers == Modifiers::NONE
                    && (matches!(key, Key::Named(Named::ArrowLeft))
                        || matches!(key, Key::Named(Named::ArrowUp)))
                {
                    let idx = self.nav.index().unwrap_or(0);
                    if idx > 0 {
                        let thumbnail_size = self.config.thumbnail_size.pixels();
                        let t = theme::active();
                        let t = t.cosmic();
                        let space_s = f32::from(t.spacing.space_s);

                        return Task::batch(vec![
                            self.update(ViewerMessage::Nav(NavMessage::GridFocus(idx - 1))),
                            self.update(ViewerMessage::Nav(NavMessage::GridScroll(
                                (idx - 1) as f32 * (thumbnail_size as f32 + space_s),
                            ))),
                            self.update(ViewerMessage::Nav(NavMessage::GridActivate(idx - 1))),
                        ]);
                    }
                } else if modifiers == Modifiers::NONE
                    && (matches!(key, Key::Named(Named::ArrowRight))
                        || matches!(key, Key::Named(Named::ArrowDown)))
                {
                    let idx = self.nav.index().unwrap_or(0);
                    if idx + 1 < self.nav.total() {
                        let thumbnail_size = self.config.thumbnail_size.pixels();
                        let t = theme::active();
                        let t = t.cosmic();
                        let space_s = f32::from(t.spacing.space_s);

                        return Task::batch(vec![
                            self.update(ViewerMessage::Nav(NavMessage::GridFocus(idx + 1))),
                            self.update(ViewerMessage::Nav(NavMessage::GridScroll(
                                (idx + 1) as f32 * (thumbnail_size as f32 + space_s),
                            ))),
                            self.update(ViewerMessage::Nav(NavMessage::GridActivate(idx + 1))),
                        ]);
                    }
                } else if let Some(msg) = keyboard_shortcut_handler(key, modifiers, text) {
                    return self.update(msg);
                }
            }
            ViewerMessage::WindowResized(size) => {
                self.window_width = Some(size.width);
                let narrow = self.is_narrow();
                if !self.is_fullscreen && narrow != self.was_narrow {
                    if narrow && self.core().nav_bar_active() {
                        self.core.nav_bar_toggle_condensed();
                    }
                    self.was_narrow = narrow;
                }
            }
            ViewerMessage::Nav(msg) => match msg {
                NavMessage::ScanComplete(dir, images, select) => {
                    self.clear_canvas();

                    self.nav.set_images(dir, images, select.as_deref());

                    if self.nav.index().is_none() && !self.nav.is_empty() {
                        self.nav.select(0);
                    }

                    self.rebuild_nav_handles();
                    if !self.nav.is_empty() {
                        let idx = self.nav.index().unwrap_or(0);
                        tasks.push(self.load_nearby_thumbnails(idx, 10));
                    }

                    // Load the selected image at full resolution.
                    if let Some(idx) = self.nav.index() {
                        if let Some(path) = self.nav.current().cloned() {
                            tasks.push(self.load_full_image(path));
                        }

                        let offset = self.grid_scroll_offset(idx);
                        tasks.push(scroll_to(
                            self.scroll_id.clone(),
                            AbsoluteOffset {
                                x: Some(0.0),
                                y: Some(offset),
                            },
                        ));
                    }
                }
                NavMessage::NavThumbnailShow(img) => {
                    if self
                        .nav_handles
                        .get(img)
                        .is_some_and(std::option::Option::is_none)
                        && let Some(path) = self.nav.images().get(img).cloned()
                    {
                        if let Some(handle) = self.cache.get_thumbnail(&path) {
                            if let Some(slot) = self.nav_handles.get_mut(img) {
                                *slot = Some(handle);
                            }
                        } else if !self.cache.is_thumbnail_pending(&path) {
                            self.cache.set_thumbnail_pending(path.clone());
                            let max_size = self.config.thumbnail_size.pixels();
                            let cache = self.cache.clone();

                            tasks.push(future(async move {
                                match load_thumbnail(path.clone(), max_size).await {
                                    Ok(loaded) => {
                                        cache.insert_thumbnail(loaded.path.clone(), loaded.handle);
                                        Action::App(ViewerMessage::Image(
                                            ImageMessage::ThumbnailReady(
                                                loaded.path,
                                                loaded.width,
                                                loaded.height,
                                            ),
                                        ))
                                    }
                                    Err(_) => Action::App(ViewerMessage::Image(
                                        ImageMessage::ThumbnailError(path),
                                    )),
                                }
                            }));
                        }
                    }
                }
                NavMessage::NavThumbnailHide(img) => {
                    if let Some(slot) = self.nav_handles.get_mut(img) {
                        *slot = None;
                    }

                    if Some(img) != self.nav.index()
                        && let Some(path) = self.nav.images().get(img)
                    {
                        self.cache.remove_full(path);
                    }
                }
                NavMessage::GridActivate(idx) => {
                    if self.nav.index() != Some(idx)
                        && self.guard_unsaved(ViewerMessage::Nav(NavMessage::GridActivate(idx)))
                    {
                        return Task::none();
                    }
                    if !self.text_editing
                        && let Some(path) = self.nav.select(idx)
                    {
                        let path = path.clone();

                        self.viewport.cancel_tool();
                        self.text_editing = false;
                        if self.color_picker.get_is_active() {
                            _ = self
                                .color_picker
                                .update::<ViewerMessage>(ColorPickerUpdate::ToggleColorPicker);
                        }

                        if self.cache.contains_full(&path) {
                            self.viewport.revert_all();
                            self.show_cached(path.clone());
                        }
                        // Don't clear the viewport - keep the previous image
                        // visible until the new one loads

                        tasks.push(self.load_full_image(path));
                        let images = self.nav.images().to_vec();
                        for (adj_idx, adj_path) in images.iter().enumerate() {
                            // distance from the just-activated image to this neighbor;
                            // skip the activated image itself, which is loaded above.
                            let dist = idx.abs_diff(adj_idx);
                            if dist != 0
                                && dist <= 1
                                && !self.cache.contains_full(adj_path)
                                && !self.cache.is_pending(adj_path)
                            {
                                tasks.push(self.load_full_image(adj_path.clone()));
                            } else if dist > 2 {
                                self.cache.remove_full(adj_path);
                            }
                        }
                    }
                }
                NavMessage::GridFocus(idx) => {
                    if let Some(p) = self.nav.select(idx) {
                        return focus(Id::new(format!("img-{}-{idx}", p.to_string_lossy())));
                    }
                }
                NavMessage::GridScroll(offset) => {
                    let thumbnail_size = self.config.thumbnail_size.pixels();

                    if let Some(cur_viewport) = self.cur_scroll.as_ref() {
                        let mut bounds = cur_viewport.bounds();
                        bounds.x = 0.;
                        let abs_offset = cur_viewport.absolute_offset();
                        bounds.y = abs_offset.y;

                        if !bounds.contains(Point::new(1., offset))
                            || !bounds.contains(Point::new(1., offset + thumbnail_size as f32))
                        {
                            tasks.push(scroll_to(
                                self.scroll_id.clone(),
                                AbsoluteOffset {
                                    x: Some(0.0),
                                    y: Some(offset),
                                },
                            ));
                        }
                    }
                }
                NavMessage::DirectoryRefreshed(images) => {
                    let old_idx = self.nav.index();
                    // WMDE: in an album of files given, the picture shown can be gone from the
                    // list; its place goes to its neighbour rather than the album shrinking to
                    // the one missing file, as `set_images` does for a picture the scan left out.
                    let selected = self
                        .nav
                        .current()
                        .cloned()
                        .filter(|path| self.chosen_files.is_none() || images.contains(path));
                    let dir = self.nav.dir().map(std::path::Path::to_path_buf);
                    if let Some(dir) = dir {
                        self.nav.set_images(dir, images, selected.as_deref());
                        if self.chosen_files.is_some()
                            && self.nav.index().is_none()
                            && !self.nav.is_empty()
                            && let Some(idx) = old_idx
                        {
                            self.nav.select(idx.min(self.nav.images().len() - 1));
                        }
                        self.rebuild_nav_handles();

                        if !self.nav.is_empty() {
                            let idx = self.nav.index().unwrap_or(0);
                            tasks.push(self.load_nearby_thumbnails(idx, 10));
                        }

                        if let Some(path) = self.nav.current().cloned()
                            && !self.cache.contains_full(&path)
                        {
                            tasks.push(self.load_full_image(path));
                        }
                    }
                }
            },
            ViewerMessage::Image(msg) => match msg {
                ImageMessage::ThumbnailReady(path, _, _) => {
                    if let Some(handle) = self.cache.get_thumbnail(&path)
                        && let Some(idx) = self.nav.images().iter().position(|pos| pos == &path)
                        && let Some(slot) = self.nav_handles.get_mut(idx)
                    {
                        *slot = Some(handle);
                    }
                }
                ImageMessage::ImageReady(path) => {
                    if self
                        .load_error
                        .as_ref()
                        .is_some_and(|(failed, _)| failed == &path)
                    {
                        self.load_error = None;
                    }
                    if let Some(idx) = self.nav.index() {
                        if self.nav.current() == Some(&path) {
                            // Clear any tool that is active when reselecting a new image.
                            if self.viewport.active_tool().is_some() {
                                self.viewport.cancel_tool();
                            }

                            // WMDE: edits made while the previous picture stood in for this
                            // one belong to that picture and do not carry over.
                            if self.canvas_path.as_ref() != Some(&path) {
                                self.viewport.revert_all();
                                self.reset_editing();
                            }
                            // Set the selected image to canvas image.
                            self.show_cached(path);
                        }
                        tasks.push(self.load_nearby_thumbnails(idx, 5));
                    }
                }
                ImageMessage::LoadError {
                    path,
                    reason,
                    detail,
                } => {
                    self.cache.clear_pending(&path);
                    // WMDE: a load started before the file last changed can fail after a
                    // later one succeeded; the cached picture is the newer word.
                    let superseded = self.cache.contains_full(&path);
                    let reported = self
                        .load_error
                        .as_ref()
                        .is_some_and(|(failed, _)| failed == &path);
                    if superseded || self.nav.current() != Some(&path) {
                        // Superseded, or a neighbour loaded ahead of time: nothing to show.
                    } else if self.canvas_path.as_ref() == Some(&path) && self.has_unsaved_edits() {
                        // WMDE: the file changed on disk under the edits and no longer reads.
                        // The canvas is this picture with the edits, which Save As can still
                        // keep, so it stays. Each rescan retries the load; one toast is enough.
                        if !reported {
                            let file = path.file_name().map_or_else(
                                || fl!("unnamed-file"),
                                |name| name.to_string_lossy().into_owned(),
                            );
                            let message =
                                fl!("error-open", file = file.as_str(), error = reason.as_str());
                            tasks.push(self.complain(
                                &format!("reload {}", path.display()),
                                &detail,
                                message,
                            ));
                            self.load_error = Some((path, reason));
                        }
                    } else {
                        // WMDE: the canvas shows the picture that failed, not the one before
                        // it, which Delete, Save and the wallpaper action would otherwise have
                        // taken for it.
                        if !reported {
                            tracing::error!("cannot open {}: {detail}", path.display());
                        }
                        self.clear_canvas();
                        self.load_error = Some((path, reason));
                    }
                }
                ImageMessage::ThumbnailError(path) => {
                    self.cache.clear_pending_thumbnail(&path);
                }
            },
            ViewerMessage::Context(page) => {
                self.context_menu_position = None;
                if self.context_page == Some(page) {
                    self.context_page = None;
                } else {
                    self.context_page = Some(page);
                }
                // WMDE: read afresh on the next opening.
                if self.context_page != Some(ContextMessage::ImageDetails) {
                    self.details = None;
                }

                self.set_show_context(self.context_page.is_some());
            }
            ViewerMessage::Canvas(msg) => match msg {
                CanvasMessage::ContextMenu(point) => {
                    self.context_menu_position = point;
                }
                CanvasMessage::ZoomIn => self.zoom_by(1.25),
                CanvasMessage::ZoomOut => self.zoom_by(1.0 / 1.25),
                CanvasMessage::ZoomBy(factor) => self.zoom_by(factor),
                CanvasMessage::Pan(pan) => {
                    let bounds = self.viewport.last_bounds().get();
                    if self.viewport.active_tool() == Some(ToolKind::Crop) {
                        let zoom = self.viewport.zoom();
                        let size = self.viewport.image_size();
                        let crop = self
                            .viewport
                            .preview_mut()
                            .and_then(|p| p.as_any_mut().downcast_mut::<CropSelection>());

                        if let Some(size) = size {
                            let fit_scale =
                                (bounds.width / size.width).min(bounds.height / size.height);
                            // Pan limit = how far the zoomed image extends past the static crop frame.
                            let (rw, rh) =
                                crop.as_ref().map_or((size.width, size.height), |crop| {
                                    (crop.region().width, crop.region().height)
                                });
                            let max_x = fit_scale * size.width.mul_add(zoom, -rw).max(0.0) / 2.0;
                            let max_y = fit_scale * size.height.mul_add(zoom, -rh).max(0.0) / 2.0;

                            let dx = pan.x.clamp(-max_x, max_x);
                            let dy = pan.y.clamp(-max_y, max_y);
                            let dv = Vector::new(dx, dy);
                            if let Some(crop) = crop {
                                // TODO do we need to update with the zoom already applied?
                                crop.pan(dv / fit_scale);
                            }

                            self.viewport.set_pan(dv);
                        }
                    } else if let Some(size) = self.viewport.image_size() {
                        let fit_scale =
                            (bounds.width / size.width).min(bounds.height / size.height);
                        let scale = self.viewport.zoom() * fit_scale;
                        // Clamp so the zoomed image can't be dragged past the view edges.
                        let max_x = size.width.mul_add(scale, -bounds.width).max(0.0) / 2.0;
                        let max_y = size.height.mul_add(scale, -bounds.height).max(0.0) / 2.0;
                        self.viewport.set_pan(Vector::new(
                            pan.x.clamp(-max_x, max_x),
                            pan.y.clamp(-max_y, max_y),
                        ));
                    } else {
                        self.viewport.set_pan(pan);
                    }
                }
                CanvasMessage::FitToView => {
                    if self.viewport.active_tool() != Some(ToolKind::Crop) {
                        self.viewport.set_zoom(1.0);
                        self.viewport.set_pan(Vector::ZERO);
                    }
                }
                CanvasMessage::ActualSize => {
                    let bounds = self.viewport.last_bounds().get();
                    let viewport_size = Size::new(bounds.width, bounds.height);
                    self.viewport.zoom_to_actual_size(viewport_size);
                }
                CanvasMessage::Fullscreen => {
                    self.is_fullscreen = !self.is_fullscreen;

                    if self.is_fullscreen {
                        self.saved_nav_toggled = self.core.nav_bar_active();
                        self.core.window.show_headerbar = false;
                        self.core.nav_bar_set_toggled(false);
                    } else {
                        self.core.window.show_headerbar = true;
                        self.core.nav_bar_set_toggled(self.saved_nav_toggled);
                    }

                    if let Some(window_id) = self.core.main_window_id() {
                        return window::set_mode(
                            window_id,
                            if self.is_fullscreen {
                                window::Mode::Fullscreen
                            } else {
                                window::Mode::Windowed
                            },
                        );
                    }
                }
                CanvasMessage::ToolStart(point) => {
                    if self.context_menu_position.is_some() {
                        self.context_menu_position = None;
                        return Task::none();
                    }

                    let s_target = self.viewport.select_target;
                    if self.viewport.move_mode {
                        let hit = self.viewport.operations_mut().iter().enumerate().rposition(
                            |(i, op)| {
                                op.movable() && op.hit_test(point, s_target.is_some_and(|s| s == i))
                            },
                        );

                        self.move_target = hit;
                        self.viewport.select_target = hit;
                        self.move_start = Some(point);
                        self.viewport.tool_dragging = true;
                        return Task::none();
                    }

                    if self.viewport.active_tool() == Some(ToolKind::Annotate)
                        && matches!(self.annotate_tool, AnnotateTool::Text)
                    {
                        if self.text_editing {
                            let on_box = self
                                .viewport
                                .preview_ref()
                                .and_then(|p| p.as_any().downcast_ref::<TextPreview>())
                                .is_some_and(|t| t.handle_at(point) != TextDragHandle::None);

                            if on_box {
                                if let Some(size) = self.viewport.image_size()
                                    && let Some(preview) = self.viewport.preview_mut()
                                {
                                    preview.on_press(point, size);
                                    self.viewport.tool_dragging = true;
                                }
                                if let Some(preview) = self.viewport.preview_mut()
                                    && let Some(t) =
                                        preview.as_any_mut().downcast_mut::<TextPreview>()
                                {
                                    if !t.sync_format_over_selection() {
                                        t.sync_format_at_cursor();
                                    }
                                    self.text_bold = t.bold;
                                    self.text_italic = t.italic;
                                    self.text_underline = t.underline;
                                    self.text_alignment = t.alignment;
                                    self.text_font_size = t.font_size;
                                    self.text_font_family = t.font_family;
                                    self.text_font_index = self
                                        .font_families
                                        .iter()
                                        .position(|&font| font == t.font_family);
                                    self.annotate_color = AnnotateColor(t.color);
                                }
                                self.sync_text_format_models();
                                return Task::none();
                            }

                            if self.color_picker.get_is_active() {
                                _ = self.color_picker.update::<ViewerMessage>(
                                    cosmic::widget::color_picker::ColorPickerUpdate::ToggleColorPicker,
                                );
                            }
                            self.viewport.apply_tool();
                            self.text_editing = false;
                            self.viewport.set_active_tool(Some(ToolKind::Annotate));
                            self.viewport.set_preview(Some(Box::new(TextPreview::new(
                                self.annotate_color.0,
                                self.text_font_size,
                                self.text_font_family,
                                self.text_bold,
                                self.text_italic,
                                self.text_underline,
                                self.text_alignment,
                                fl!("type-here"),
                            ))));
                            self.viewport.rebuild_display();
                            return Task::none();
                        }

                        let re_edit = {
                            let ops = self.viewport.operations_mut();
                            let hit = ops.iter().rposition(|op| {
                                op.as_any()
                                    .downcast_ref::<TextOperation>()
                                    .is_some_and(|t| t.hit_test(point))
                            });

                            hit.and_then(|idx| {
                                let op = ops.remove(idx);
                                op.as_any()
                                    .downcast_ref::<TextOperation>()
                                    .map(viewer_tools::annotate::TextOperation::to_preview)
                            })
                        };

                        if let Some(preview) = re_edit {
                            self.text_bold = preview.bold;
                            self.text_italic = preview.italic;
                            self.text_underline = preview.underline;
                            self.text_font_size = preview.font_size;
                            self.text_font_family = preview.font_family;
                            self.text_alignment = preview.alignment;
                            self.annotate_color = AnnotateColor(preview.color);
                            self.text_editing = true;
                            self.viewport.set_preview(Some(Box::new(preview)));
                            self.viewport.rebuild_display();
                            self.sync_text_format_models();
                            return Task::none();
                        }

                        self.viewport.set_preview(Some(Box::new(TextPreview::new(
                            self.annotate_color.0,
                            self.text_font_size,
                            self.text_font_family,
                            self.text_bold,
                            self.text_italic,
                            self.text_underline,
                            self.text_alignment,
                            fl!("type-here"),
                        ))));
                    }

                    if let Some(size) = self.viewport.image_size()
                        && let Some(preview) = self.viewport.preview_mut()
                    {
                        preview.on_press(point, size);
                        self.viewport.tool_dragging = true;
                    }
                }
                CanvasMessage::ToolDrag(point) => {
                    if self.viewport.move_mode {
                        if let Some(idx) = self.move_target
                            && let Some(start) = self.move_start
                        {
                            let dx = point.x - start.x;
                            let dy = point.y - start.y;
                            if let Some(op) = self.viewport.operations_mut().get_mut(idx) {
                                op.translate(dx, dy);
                            }
                            self.move_start = Some(point);
                        }
                    } else if let Some(size) = self.viewport.image_size()
                        && let Some(preview) = self.viewport.preview_mut()
                    {
                        preview.on_drag(point, size);
                    }

                    if let Some(preview) = self.viewport.preview_mut()
                        && let Some(t) = preview.as_any_mut().downcast_mut::<TextPreview>()
                    {
                        if !t.sync_format_over_selection() {
                            t.sync_format_at_cursor();
                        }
                        self.text_bold = t.bold;
                        self.text_italic = t.italic;
                        self.text_underline = t.underline;
                        self.text_alignment = t.alignment;
                    } else {
                        self.sync_text_format_models();
                    }
                }
                CanvasMessage::ToolEnd => {
                    if self.viewport.move_mode {
                        // TODO if have move target and toolbar selects color, size?, or text formatting, update the move target...
                        self.move_target = None;
                        self.move_start = None;
                        self.viewport.tool_dragging = false;
                        return Task::none();
                    }
                    self.viewport.tool_dragging = false;
                    if let Some(size) = self.viewport.image_size()
                        && let Some(preview) = self.viewport.preview_mut()
                    {
                        preview.on_release(Point::ORIGIN, size);
                    }

                    if let Some(preview) = self.viewport.preview_mut()
                        && let Some(t) = preview.as_any_mut().downcast_mut::<TextPreview>()
                    {
                        if !t.sync_format_over_selection() {
                            t.sync_format_at_cursor();
                        }
                        self.text_bold = t.bold;
                        self.text_italic = t.italic;
                        self.text_underline = t.underline;
                        self.text_alignment = t.alignment;
                    } else {
                        self.sync_text_format_models();
                    }

                    // Crop is applied only via Apply/Enter, never on drag-release, so the frame
                    // stays live for further adjust/pan and behaves the same in Custom and ratio modes.

                    // Set text_editing flag when text preview enters editing mode
                    if self.viewport.active_tool() == Some(ToolKind::Annotate)
                        && matches!(self.annotate_tool, AnnotateTool::Text)
                        && let Some(preview) = self.viewport.preview_mut()
                        && let Some(text_preview) =
                            preview.as_any_mut().downcast_mut::<TextPreview>()
                    {
                        self.text_editing = text_preview.is_editing();
                    }

                    // Auto-commit for annotation, each stroke goes on the undo stack
                    if self.viewport.active_tool() == Some(ToolKind::Annotate)
                        && !matches!(self.annotate_tool, AnnotateTool::Text)
                    {
                        self.viewport.apply_tool();
                        self.viewport.set_active_tool(Some(ToolKind::Annotate));
                        let preview: Box<dyn ToolOperation> = match self.annotate_tool {
                            AnnotateTool::Highlighter => Box::new(HighlighterPreview::new(
                                self.annotate_color.0,
                                self.annotate_stroke_size,
                            )),
                            AnnotateTool::Rectangle
                            | AnnotateTool::Ellipse
                            | AnnotateTool::Block
                            | AnnotateTool::Line
                            | AnnotateTool::Arrow
                            | AnnotateTool::Star
                            | AnnotateTool::Polygon => {
                                let kind = match self.annotate_tool {
                                    AnnotateTool::Rectangle => ShapeKind::Rectangle,
                                    AnnotateTool::Ellipse => ShapeKind::Ellipse,
                                    AnnotateTool::Block => ShapeKind::Block,
                                    AnnotateTool::Line => ShapeKind::Line,
                                    AnnotateTool::Arrow => ShapeKind::Arrow,
                                    AnnotateTool::Star => ShapeKind::Star,
                                    AnnotateTool::Polygon => ShapeKind::Polygon,
                                    _ => unreachable!(),
                                };
                                Box::new(ShapePreview::new(
                                    kind,
                                    self.annotate_color.0,
                                    self.annotate_stroke_size,
                                ))
                            }
                            _ => Box::new(PenPreview::new(
                                self.annotate_color.0,
                                self.annotate_stroke_size,
                            )),
                        };
                        self.viewport.set_preview(Some(preview));
                    }
                }
            },
            ViewerMessage::Edit(msg) => {
                self.context_menu_position = None;
                if !matches!(msg, EditMessage::ColorPicker(_)) && self.color_picker.get_is_active()
                {
                    if let Some(color) = self.color_picker.get_applied_color() {
                        self.annotate_color = AnnotateColor(color);
                        // WMDE: `get_mut`, not indexing: a stale index panicked, and the
                        // release profile aborts on panic.
                        if let Some(op) = self
                            .viewport
                            .select_target
                            .and_then(|target| self.viewport.operations_mut().get_mut(target))
                        {
                            op.set_color(color);
                        }

                        self.save_last_color();
                    }
                    _ = self.color_picker.update::<ViewerMessage>(
                        cosmic::widget::color_picker::ColorPickerUpdate::ToggleColorPicker,
                    );
                }
                match msg {
                    EditMessage::Annotate => {
                        if self.viewport.active_tool() != Some(ToolKind::Annotate) {
                            self.viewport.set_active_tool(Some(ToolKind::Annotate));
                            return self.update(ViewerMessage::Edit(EditMessage::AnnotateTool(
                                self.annotate_tool,
                            )));
                        }
                    }
                    EditMessage::AnnotateApply => {
                        self.viewport.apply_tool();
                        self.viewport.set_active_tool(None);
                        self.viewport.set_preview(None);
                        self.text_editing = false;
                        self.show_text_format_menu = false;
                        self.viewport.tool_dragging = false;
                        self.viewport.move_mode = false;
                    }
                    EditMessage::AnnotateCancel => {
                        if self.text_editing {
                            if self.color_picker.get_is_active() {
                                _ = self
                                    .color_picker
                                    .update::<ViewerMessage>(ColorPickerUpdate::ToggleColorPicker);
                            }
                            self.text_editing = false;
                        }
                        self.viewport.cancel_tool();
                        self.viewport.revert_all();
                        self.viewport.set_active_tool(None);
                        self.viewport.set_preview(None);
                        self.viewport.move_mode = false;
                        // Restore working image from cache so rotation etc. still works
                        if let Some(path) = self.nav.current().cloned() {
                            self.show_cached(path);
                        }
                    }
                    EditMessage::AnnotateStroke(size) => {
                        if self.annotate_tool == AnnotateTool::Highlighter {
                            let sizes: [f32; 7] = [8., 10., 12., 14., 16., 18., 20.];

                            if let Some((&size, select_target)) =
                                sizes.get(size).zip(self.viewport.select_target)
                            {
                                if let Some(op) =
                                    self.viewport.operations_mut().get_mut(select_target)
                                {
                                    op.set_annotation_stroke(size);
                                }

                                self.annotate_stroke_size = size;
                                if let Some(highlighter) =
                                    self.viewport.preview_mut().and_then(|preview| {
                                        preview.as_any_mut().downcast_mut::<HighlighterPreview>()
                                    })
                                {
                                    highlighter.width = size;
                                }
                            }
                        } else {
                            let sizes: [f32; 9] = [2., 4., 6., 8., 10., 12., 16., 24., 32.];
                            if let Some(&size) = sizes.get(size) {
                                if let Some(o) = self
                                    .viewport
                                    .select_target
                                    .and_then(|i| self.viewport.operations_mut().get_mut(i))
                                {
                                    o.set_annotation_stroke(size);
                                }

                                self.annotate_stroke_size = size;
                                if let Some(preview) = self.viewport.preview_mut() {
                                    if let Some(pen) =
                                        preview.as_any_mut().downcast_mut::<PenPreview>()
                                    {
                                        pen.width = size;
                                    } else if let Some(shape) =
                                        preview.as_any_mut().downcast_mut::<ShapePreview>()
                                    {
                                        shape.width = size;
                                    }
                                }
                            }
                        }
                    }
                    EditMessage::AnnotateTool(tool) => {
                        if matches!(self.annotate_tool, AnnotateTool::Text)
                            && !matches!(tool, AnnotateTool::Text)
                        {
                            if self
                                .viewport
                                .preview_mut()
                                .is_some_and(|p| p.as_any().downcast_ref::<TextPreview>().is_some())
                            {
                                self.viewport.apply_tool_continue_edit();
                            }
                            self.text_editing = false;
                        }
                        self.viewport.select_target = None;
                        self.annotate_tool = tool;
                        self.show_text_format_menu = false;
                        self.viewport.move_mode = false;

                        match tool {
                            AnnotateTool::Highlighter => {
                                self.viewport
                                    .set_preview(Some(Box::new(HighlighterPreview::new(
                                        self.annotate_color.0,
                                        self.annotate_stroke_size,
                                    ))));
                            }
                            AnnotateTool::Pen => {
                                self.viewport.set_preview(Some(Box::new(PenPreview::new(
                                    self.annotate_color.0,
                                    self.annotate_stroke_size,
                                ))));
                            }
                            AnnotateTool::Rectangle
                            | AnnotateTool::Ellipse
                            | AnnotateTool::Line
                            | AnnotateTool::Arrow
                            | AnnotateTool::Star
                            | AnnotateTool::Block
                            | AnnotateTool::Polygon => {
                                if self.shape_popup {
                                    self.shape_popup = false;
                                }
                                let kind = match tool {
                                    AnnotateTool::Rectangle => ShapeKind::Rectangle,
                                    AnnotateTool::Block => ShapeKind::Block,
                                    AnnotateTool::Ellipse => ShapeKind::Ellipse,
                                    AnnotateTool::Line => ShapeKind::Line,
                                    AnnotateTool::Arrow => ShapeKind::Arrow,
                                    AnnotateTool::Star => ShapeKind::Star,
                                    AnnotateTool::Polygon => ShapeKind::Polygon,
                                    _ => unreachable!(),
                                };
                                self.selected_shape = tool;
                                self.viewport.set_preview(Some(Box::new(ShapePreview::new(
                                    kind,
                                    self.annotate_color.0,
                                    self.annotate_stroke_size,
                                ))));
                            }
                            AnnotateTool::Text => {
                                self.viewport.set_preview(Some(Box::new(TextPreview::new(
                                    self.annotate_color.0,
                                    self.text_font_size,
                                    self.text_font_family,
                                    self.text_bold,
                                    self.text_italic,
                                    self.text_underline,
                                    self.text_alignment,
                                    fl!("type-here"),
                                ))));
                                self.viewport.set_active_tool(Some(ToolKind::Annotate));
                            }
                        }
                    }
                    EditMessage::AnnotateColor(color) => {
                        self.annotate_color = color;
                        self.save_last_color();
                        // TODO use active selection
                        if let Some(o) = self
                            .viewport
                            .select_target
                            .and_then(|i| self.viewport.operations_mut().get_mut(i))
                        {
                            if let Some(text) = o.as_any_mut().downcast_mut::<TextOperation>() {
                                if let Some(op) = text.as_any_mut().downcast_mut::<TextOperation>()
                                {
                                    let mut p = op.to_preview();
                                    let c = color.0;

                                    p.select_all();
                                    p.apply_attr_to_selection(|a| a.color(text_color_rgba(c)));

                                    if let Some(committed) = p.commit() {
                                        *o = committed;
                                    }
                                }
                            } else {
                                o.set_color(color.0);
                            }
                        }

                        // Update the active preview's color if one exists
                        if let Some(preview) = self.viewport.preview_mut() {
                            if let Some(pen) = preview.as_any_mut().downcast_mut::<PenPreview>() {
                                pen.color = color.0;
                            } else if let Some(highlighter) =
                                preview.as_any_mut().downcast_mut::<HighlighterPreview>()
                            {
                                highlighter.color = color.0;
                            } else if let Some(shape) =
                                preview.as_any_mut().downcast_mut::<ShapePreview>()
                            {
                                shape.color = color.0;
                            } else if let Some(text) =
                                preview.as_any_mut().downcast_mut::<TextPreview>()
                            {
                                text.color = color.0;
                                if text.has_selection() {
                                    let c = color.0;
                                    text.apply_attr_to_selection(|a| a.color(text_color_rgba(c)));
                                }
                            }
                        }
                        self.viewport.rebuild_display();
                    }
                    EditMessage::Crop => {
                        if self.viewport.active_tool() != Some(ToolKind::Crop) {
                            self.viewport.set_zoom(1.0);
                            self.viewport.set_pan(Vector::ZERO);
                            self.viewport.set_active_tool(Some(ToolKind::Crop));
                            self.crop_ratio = CropRatio::Custom;
                            self.viewport
                                .set_preview(Some(Box::new(CropSelection::new())));
                        }
                    }
                    EditMessage::CropApply => {
                        let fit_region = self
                            .viewport
                            .preview_mut()
                            .and_then(|preview| {
                                preview.as_any_mut().downcast_mut::<CropSelection>()
                            })
                            .map(|sel| sel.region());

                        if let Some(fit_region) = fit_region
                            && let Some(size) = self.viewport.image_size()
                        {
                            let zoom = self.viewport.zoom();
                            let pan = self.viewport.pan();
                            let bounds = self.viewport.last_bounds().get();
                            let fit_scale =
                                (bounds.width / size.width).min(bounds.height / size.height);

                            // default (unzoomed, un-panned) view: zoom is set to exactly 1.0 on reset.
                            let region = if (zoom - 1.0).abs() < f32::EPSILON && pan == Vector::ZERO
                            {
                                fit_region
                            } else {
                                let cx = size.width / 2.0;
                                let cy = size.height / 2.0;
                                Rectangle::new(
                                    Point::new(
                                        fit_region.x / zoom + cx * (1.0 - 1.0 / zoom)
                                            - pan.x / (zoom * fit_scale),
                                        fit_region.y / zoom + cy * (1.0 - 1.0 / zoom)
                                            - pan.y / (zoom * fit_scale),
                                    ),
                                    Size::new(fit_region.width / zoom, fit_region.height / zoom),
                                )
                            };

                            // crop_to_region casts to u32 unclamped; clamp the off-image case.
                            let x = region.x.clamp(0.0, size.width);
                            let y = region.y.clamp(0.0, size.height);
                            let w = region.width.min(size.width - x).max(1.0);
                            let h = region.height.min(size.height - y).max(1.0);
                            let region = Rectangle::new(Point::new(x, y), Size::new(w, h));

                            for op in self.viewport.operations_mut() {
                                op.transform_crop(region);
                            }

                            self.viewport.apply_tool();

                            self.viewport
                                .map_working_image(|img| crop_to_region(img, region));
                            self.viewport.rebuild_display();
                            self.viewport.set_zoom(1.0);
                            self.viewport.set_pan(Vector::ZERO);
                        }
                    }
                    EditMessage::CropCancel => {
                        self.viewport.cancel_tool();
                        if let Some(current) = self.nav.current() {
                            self.viewport.revert_all();
                            self.cache.remove_full(current);
                            tasks.push(self.load_full_image(current.clone()));
                        }
                    }
                    EditMessage::CropRatioPopupToggle => {
                        self.crop_ratio_popup = !self.crop_ratio_popup;
                    }
                    EditMessage::CropRatio(ratio) => {
                        let img_size = self.viewport.image_size();
                        if let Some(preview) = self.viewport.preview_mut()
                            && let Some(crop) = preview.as_any_mut().downcast_mut::<CropSelection>()
                            && let Some(size) = img_size
                        {
                            crop.set_ratio(ratio, size);

                            self.crop_ratio = ratio;
                            return self.update(ViewerMessage::Canvas(CanvasMessage::Pan(
                                Default::default(),
                            )));
                        }
                    }
                    EditMessage::RotateLeft => {
                        let bounds = self.viewport.last_bounds().get();
                        let viewport_size = Size::new(bounds.width, bounds.height);
                        let percent = self.viewport.actual_percent(viewport_size);
                        let is_cropping = self.viewport.active_tool() == Some(ToolKind::Crop);
                        // Commit an in-progress annotation (e.g. a text box being edited) so it
                        // rotates with the image instead of being discarded; `apply_tool` is a
                        // no-op (returns false) for an empty preview, which then cancels.
                        if !is_cropping && !self.viewport.apply_tool() {
                            self.viewport.cancel_tool();
                        }
                        let direction = RotateDirection::Left;
                        let image_size = self.viewport.image_size().unwrap_or(Size::ZERO);

                        for op in self.viewport.operations_mut() {
                            op.transform_rotate(direction, image_size);
                        }

                        self.viewport
                            .commit(Box::new(RotateOperation::new(direction)));

                        self.viewport.map_working_image(DynamicImage::rotate270);
                        self.viewport.rebuild_display();
                        self.viewport.set_actual_percent(percent, viewport_size);

                        if is_cropping {
                            let new_size = self.viewport.image_size().unwrap_or(Size::ZERO);
                            if let Some(preview) = self.viewport.preview_mut()
                                && let Some(crop) =
                                    preview.as_any_mut().downcast_mut::<CropSelection>()
                            {
                                crop.activate(self.crop_ratio, new_size);
                                return self.update(ViewerMessage::Canvas(CanvasMessage::Pan(
                                    Default::default(),
                                )));
                            }
                        }
                    }
                    EditMessage::RotateRight => {
                        let bounds = self.viewport.last_bounds().get();
                        let viewport_size = Size::new(bounds.width, bounds.height);
                        let percent = self.viewport.actual_percent(viewport_size);
                        let is_cropping = self.viewport.active_tool() == Some(ToolKind::Crop);
                        // Commit an in-progress annotation (e.g. a text box being edited) so it
                        // rotates with the image instead of being discarded; `apply_tool` is a
                        // no-op (returns false) for an empty preview, which then cancels.
                        if !is_cropping && !self.viewport.apply_tool() {
                            self.viewport.cancel_tool();
                        }
                        let direction = RotateDirection::Right;
                        let image_size = self.viewport.image_size().unwrap_or(Size::ZERO);

                        for op in self.viewport.operations_mut() {
                            op.transform_rotate(direction, image_size);
                        }

                        self.viewport
                            .commit(Box::new(RotateOperation::new(direction)));

                        self.viewport.map_working_image(DynamicImage::rotate90);
                        self.viewport.rebuild_display();
                        self.viewport.set_actual_percent(percent, viewport_size);

                        if is_cropping {
                            let new_size = self.viewport.image_size().unwrap_or(Size::ZERO);
                            if let Some(preview) = self.viewport.preview_mut()
                                && let Some(crop) =
                                    preview.as_any_mut().downcast_mut::<CropSelection>()
                            {
                                crop.activate(self.crop_ratio, new_size);
                                return self.update(ViewerMessage::Canvas(CanvasMessage::Pan(
                                    Default::default(),
                                )));
                            }
                        }
                    }
                    EditMessage::ShapePopupOpen => self.shape_popup = true,
                    EditMessage::ShapePopupClose => self.shape_popup = false,
                    EditMessage::StrokePopupToggle => self.stroke_popup = !self.stroke_popup,
                    EditMessage::ColorPicker(update) => {
                        let was_active = self.color_picker.get_is_active();

                        match &update {
                            AppliedColor => {
                                tasks.push(
                                    self.color_picker
                                        .update::<ViewerMessage>(update)
                                        .map(|_| Action::None),
                                );
                                if let Some(color) = self.color_picker.get_applied_color() {
                                    self.annotate_color = AnnotateColor(color);
                                    self.save_last_color();
                                }
                                if self.color_picker.get_is_active() {
                                    _ = self
                                        .color_picker
                                        .update::<ViewerMessage>(ToggleColorPicker);
                                }
                            }
                            Cancel => {
                                tasks.push(
                                    self.color_picker
                                        .update::<ViewerMessage>(update)
                                        .map(|_| Action::None),
                                );
                                if self.color_picker.get_is_active() {
                                    _ = self
                                        .color_picker
                                        .update::<ViewerMessage>(ToggleColorPicker);
                                }
                            }
                            ToggleColorPicker => {
                                // Closing via toggle - apply the color
                                if was_active
                                    && let Some(color) = self.color_picker.get_applied_color()
                                {
                                    self.annotate_color = AnnotateColor(color);
                                    self.save_last_color();
                                }
                                tasks.push(
                                    self.color_picker
                                        .update::<ViewerMessage>(update)
                                        .map(|_| Action::None),
                                );
                            }
                            ColorPickerUpdate::ActionFinished => {
                                if let Some(color) = self.color_picker.get_applied_color() {
                                    self.annotate_color = AnnotateColor(color);
                                    self.has_custom_color = true;
                                }

                                tasks.push(
                                    self.color_picker
                                        .update::<ViewerMessage>(update)
                                        .map(|_| Action::None),
                                );

                                // ActionFinished auto-closes the picker in the model;
                                // reopen it so the user can keep adjusting until they
                                // are explicitly done.
                                if !self.color_picker.get_is_active() {
                                    _ = self
                                        .color_picker
                                        .update::<ViewerMessage>(ToggleColorPicker);
                                }
                            }
                            _ => {
                                tasks.push(
                                    self.color_picker
                                        .update::<ViewerMessage>(update)
                                        .map(|_| Action::None),
                                );
                            }
                        }

                        if let Some(preview) = self.viewport.preview_mut() {
                            if let Some(text) = preview.as_any_mut().downcast_mut::<TextPreview>() {
                                text.color = self.annotate_color.0;
                                if text.has_selection() {
                                    let c = self.annotate_color.0;
                                    text.apply_attr_to_selection(|a| a.color(text_color_rgba(c)));
                                }
                            } else if let Some(pen) =
                                preview.as_any_mut().downcast_mut::<PenPreview>()
                            {
                                pen.color = self.annotate_color.0;
                            } else if let Some(highlighter) =
                                preview.as_any_mut().downcast_mut::<HighlighterPreview>()
                            {
                                highlighter.color = self.annotate_color.0;
                            } else if let Some(shape) =
                                preview.as_any_mut().downcast_mut::<ShapePreview>()
                            {
                                shape.color = self.annotate_color.0;
                            }
                        }
                        self.viewport.rebuild_display();
                    }
                    EditMessage::ToggleTextFormatMenu => {
                        self.show_text_format_menu = !self.show_text_format_menu;
                    }
                    EditMessage::TextBold => {
                        // if we have an operation, convert to preview, select all, apply change, and then
                        self.text_bold = !self.text_bold;
                        let bold = self.text_bold;

                        if let Some(d) = self
                            .viewport
                            .select_target
                            .and_then(|i| self.viewport.operations_mut().get_mut(i))
                        {
                            if let Some(o) = d.as_any_mut().downcast_mut::<TextOperation>() {
                                let mut p = o.to_preview();
                                p.select_all();
                                p.apply_attr_to_selection(|a| {
                                    a.weight(if bold {
                                        cosmic_text::Weight::BOLD
                                    } else {
                                        cosmic_text::Weight::NORMAL
                                    })
                                });
                                if let Some(committed) = p.commit() {
                                    *d = committed;
                                }
                            }
                        }

                        if let Some(preview) = self.viewport.preview_mut()
                            && let Some(text) = preview.as_any_mut().downcast_mut::<TextPreview>()
                        {
                            if text.has_selection() {
                                text.apply_attr_to_selection(|a| {
                                    a.weight(if bold {
                                        cosmic_text::Weight::BOLD
                                    } else {
                                        cosmic_text::Weight::NORMAL
                                    })
                                });
                            }
                            text.bold = self.text_bold;
                        }
                        self.viewport.rebuild_display();
                        self.sync_text_format_models();
                    }
                    EditMessage::TextItalic => {
                        self.text_italic = !self.text_italic;
                        let italic = self.text_italic;

                        if let Some(d) = self
                            .viewport
                            .select_target
                            .and_then(|i| self.viewport.operations_mut().get_mut(i))
                        {
                            if let Some(o) = d.as_any_mut().downcast_mut::<TextOperation>() {
                                let mut p = o.to_preview();
                                p.select_all();
                                p.apply_attr_to_selection(|a| {
                                    a.style(if italic {
                                        cosmic_text::Style::Italic
                                    } else {
                                        cosmic_text::Style::Normal
                                    })
                                });
                                if let Some(committed) = p.commit() {
                                    *d = committed;
                                }
                            }
                        }

                        if let Some(preview) = self.viewport.preview_mut()
                            && let Some(text) = preview.as_any_mut().downcast_mut::<TextPreview>()
                        {
                            if text.has_selection() {
                                text.apply_attr_to_selection(|a| {
                                    a.style(if italic {
                                        cosmic_text::Style::Italic
                                    } else {
                                        cosmic_text::Style::Normal
                                    })
                                });
                            }
                            text.italic = self.text_italic;
                        }
                        self.viewport.rebuild_display();
                        self.sync_text_format_models();
                    }
                    EditMessage::TextUnderline => {
                        self.text_underline = !self.text_underline;
                        let underline = self.text_underline;

                        if let Some(d) = self
                            .viewport
                            .select_target
                            .and_then(|i| self.viewport.operations_mut().get_mut(i))
                        {
                            if let Some(o) = d.as_any_mut().downcast_mut::<TextOperation>() {
                                let mut p = o.to_preview();
                                p.select_all();
                                p.apply_attr_to_selection(|a| a.metadata(usize::from(underline)));
                                if let Some(committed) = p.commit() {
                                    *d = committed;
                                }
                            }
                        }

                        if let Some(preview) = self.viewport.preview_mut()
                            && let Some(text) = preview.as_any_mut().downcast_mut::<TextPreview>()
                        {
                            if text.has_selection() {
                                text.apply_attr_to_selection(|a| {
                                    a.metadata(usize::from(underline))
                                });
                            }
                            text.underline = self.text_underline;
                        }
                        self.viewport.rebuild_display();
                        self.sync_text_format_models();
                    }
                    EditMessage::TextAlignment(alignment) => {
                        self.text_alignment = alignment;

                        if let Some(d) = self
                            .viewport
                            .select_target
                            .and_then(|i| self.viewport.operations_mut().get_mut(i))
                        {
                            if let Some(o) = d.as_any_mut().downcast_mut::<TextOperation>() {
                                let mut p = o.to_preview();
                                p.set_line_alignment(alignment);
                                if let Some(committed) = p.commit() {
                                    *d = committed;
                                }
                            }
                        }

                        if let Some(preview) = self.viewport.preview_mut()
                            && let Some(text) = preview.as_any_mut().downcast_mut::<TextPreview>()
                        {
                            text.set_line_alignment(alignment);
                        }
                        self.viewport.rebuild_display();
                        self.sync_text_format_models();
                    }
                    EditMessage::TextFontSize(idx) => {
                        if let Some(&size_pt) = FONT_SIZE_PRESETS_PT.get(idx) {
                            let size_px = pt_to_px(size_pt);

                            if let Some(d) = self
                                .viewport
                                .select_target
                                .and_then(|i| self.viewport.operations_mut().get_mut(i))
                            {
                                if let Some(o) = d.as_any_mut().downcast_mut::<TextOperation>() {
                                    let mut p = o.to_preview();
                                    p.select_all();
                                    p.update_font_size(size_px);
                                    if let Some(committed) = p.commit() {
                                        *d = committed;
                                    }
                                }
                            }

                            self.text_font_size = size_px;
                            if let Some(preview) = self.viewport.preview_mut()
                                && let Some(text) =
                                    preview.as_any_mut().downcast_mut::<TextPreview>()
                            {
                                text.update_font_size(size_px);
                            }
                            self.viewport.rebuild_display();
                        }
                    }
                    EditMessage::TextFontFamily(idx) => {
                        self.text_font_index = Some(idx);
                        let fam = self.font_families[idx];
                        self.text_font_family = fam;

                        if let Some(d) = self
                            .viewport
                            .select_target
                            .and_then(|i| self.viewport.operations_mut().get_mut(i))
                        {
                            if let Some(o) = d.as_any_mut().downcast_mut::<TextOperation>() {
                                let mut p = o.to_preview();
                                p.select_all();
                                p.apply_attr_to_selection(|a| {
                                    a.family(cosmic_text::Family::Name(fam))
                                });
                                if let Some(committed) = p.commit() {
                                    *d = committed;
                                }
                            }
                        }

                        if let Some(preview) = self.viewport.preview_mut()
                            && let Some(text) = preview.as_any_mut().downcast_mut::<TextPreview>()
                        {
                            if text.has_selection() {
                                text.apply_attr_to_selection(|a| {
                                    a.family(cosmic_text::Family::Name(fam))
                                });
                            }
                            text.font_family = fam;
                        }
                        self.viewport.rebuild_display();
                    }
                    EditMessage::TextStyleActivated(entity) => {
                        let msg = match self.text_style_model.data::<TextStyle>(entity).copied() {
                            Some(TextStyle::Bold) => Some(EditMessage::TextBold),
                            Some(TextStyle::Italic) => Some(EditMessage::TextItalic),
                            Some(TextStyle::Underline) => Some(EditMessage::TextUnderline),
                            None => None,
                        };

                        if let Some(msg) = msg {
                            return self.update(ViewerMessage::Edit(msg));
                        }
                    }
                    EditMessage::TextAlignActivated(entity) => {
                        if let Some(alignment) =
                            self.text_align_model.data::<Horizontal>(entity).copied()
                        {
                            return self.update(ViewerMessage::Edit(EditMessage::TextAlignment(
                                alignment,
                            )));
                        }
                    }
                    EditMessage::TextCancel => {
                        self.viewport.cancel_tool();
                        self.viewport.select_target = None;
                        self.text_editing = false;
                    }
                    EditMessage::TextApply => {
                        self.viewport.apply_tool();
                        self.viewport.select_target = None;
                        self.text_editing = false;
                    }
                    EditMessage::ToggleMoveMode => {
                        self.viewport.move_mode = !self.viewport.move_mode;
                        self.move_target = None;
                        self.viewport.select_target = None;
                        self.move_start = None;
                    }
                    EditMessage::Undo => {
                        self.viewport.select_target = None;
                        if let Some(op) = self.viewport.undo() {
                            if let Some(rotate) = op.as_any().downcast_ref::<RotateOperation>() {
                                let inverse = rotate.direction.inverse();
                                let image_size = self.viewport.image_size().unwrap_or(Size::ZERO);

                                for op in self.viewport.operations_mut() {
                                    op.transform_rotate(inverse, image_size);
                                }

                                self.viewport.map_working_image(|img| match inverse {
                                    RotateDirection::Left => img.rotate270(),
                                    RotateDirection::Right => img.rotate90(),
                                });
                                self.rebuild_working_image();
                            } else if let Some(crop) = op.as_any().downcast_ref::<CropOperation>() {
                                let region = crop.region;

                                for op in self.viewport.operations_mut() {
                                    op.transform_crop(Rectangle::new(
                                        Point::new(-region.x, -region.y),
                                        Size::ZERO,
                                    ));
                                }

                                self.rebuild_working_image();

                                // Reset the crop preview to the restored image size
                                if let Some(new_size) = self.viewport.image_size()
                                    && let Some(preview) = self.viewport.preview_mut()
                                    && let Some(selection) =
                                        preview.as_any_mut().downcast_mut::<CropSelection>()
                                {
                                    let ratio = selection.ratio;
                                    selection.activate(ratio, new_size);
                                    return self.update(ViewerMessage::Canvas(CanvasMessage::Pan(
                                        Default::default(),
                                    )));
                                }
                            } else {
                                self.viewport.rebuild_display();
                            }
                        }
                    }
                    EditMessage::Redo => {
                        self.viewport.select_target = None;

                        if let Some(op) = self.viewport.redo() {
                            if let Some(rotate) = op.as_any().downcast_ref::<RotateOperation>() {
                                let direction = rotate.direction;
                                let image_size = self.viewport.image_size().unwrap_or(Size::ZERO);

                                for op in self.viewport.operations_mut() {
                                    op.transform_rotate(direction, image_size);
                                }

                                self.rebuild_working_image();
                            } else if let Some(crop) = op.as_any().downcast_ref::<CropOperation>() {
                                let region = crop.region;

                                for op in self.viewport.operations_mut() {
                                    op.transform_crop(region);
                                }

                                self.rebuild_working_image();
                            } else {
                                self.viewport.rebuild_display();
                            }
                        }
                    }
                    EditMessage::RevertAll => {
                        self.viewport.select_target = None;
                        self.viewport.revert_all()
                    }
                }
            }
            ViewerMessage::Surface(action) => {
                return cosmic::task::message(cosmic::Action::Surface(action));
            }
            ViewerMessage::ShowNavbar(show_navbar) => {
                config_set!(show_navbar, show_navbar);
            }
            ViewerMessage::NavScroll(viewport) => {
                self.cur_scroll = Some(viewport);
            }
            ViewerMessage::LaunchUrl(url) => {
                if let Err(err) = open::that_detached(&url) {
                    tasks.push(self.complain(
                        &format!("open {url}"),
                        &err.to_string(),
                        fl!("error-open-link"),
                    ));
                }
            }
            ViewerMessage::HoverItem(img) => {
                self.hovered_item = img;
            }
        }

        if tasks.is_empty() {
            Task::none()
        } else {
            Task::batch(tasks)
        }
    }

    fn on_app_exit(&mut self) -> Option<Self::Message> {
        Some(ViewerMessage::CloseRequested)
    }

    fn on_close_requested(&self, id: window::Id) -> Option<Self::Message> {
        (self.core().main_window_id() == Some(id)).then_some(ViewerMessage::Quit)
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        let watcher_sub =
            crate::watcher::watch_directories(self.watched_dirs()).map(ViewerMessage::WatcherEvent);

        // WMDE: the details drawer reads the file it describes off the drawing thread, once for
        // each picture and again whenever the file changes. A plain thread, as for
        // OpenContaining: a read stuck on a dead network mount must not hold the exit.
        let details_sub = match (self.context_page, self.nav.current()) {
            (Some(ContextMessage::ImageDetails), Some(path)) => {
                Subscription::run_with((path.clone(), self.details_stamp), |(path, stamp)| {
                    let (path, stamp) = (path.clone(), *stamp);
                    cosmic::iced::futures::stream::once(async move {
                        let (sender, receiver) = tokio::sync::oneshot::channel();
                        let file = path.clone();
                        std::thread::spawn(move || {
                            _ = sender.send(read_details(&file));
                        });
                        let details = receiver.await.unwrap_or_default();
                        ViewerMessage::DetailsRead(path, stamp, Box::new(details))
                    })
                })
            }
            _ => Subscription::none(),
        };

        Subscription::batch([
            event::listen_with(|event, status, _id| match event {
                iced::Event::Window(iced::window::Event::Resized(size)) => {
                    Some(ViewerMessage::WindowResized(size))
                }
                iced::Event::Window(iced::window::Event::CloseRequested) => {
                    Some(ViewerMessage::CloseRequested)
                }
                // WMDE: a key a widget took - typing in the colour picker's hex field - is
                // not a shortcut too; Delete there moved the picture to the trash.
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key,
                    modifiers,
                    text,
                    ..
                }) if status == event::Status::Ignored => {
                    Some(ViewerMessage::KeyPressed(key, modifiers, text))
                }
                _ => None,
            }),
            watcher_sub,
            details_sub,
        ])
    }
}

// HELPER FUNCTIONS

fn detail_row<'a>(label: String, value: String) -> Element<'a, ViewerMessage> {
    Column::new()
        .push(text::caption(label))
        .push(text::body(value))
        .spacing(2)
        .into()
}

/// WMDE: the files among `paths`, each once, the first spelling of it kept: the same file given
/// twice, or under two names, is one picture of the album.
fn given_files(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    paths
        .iter()
        .filter(|path| path.is_file())
        .filter(|path| seen.insert(std::fs::canonicalize(path).unwrap_or_else(|_| (*path).clone())))
        .cloned()
        .collect()
}

fn friendly_type_name(ext: &str) -> String {
    let format = match ext.to_lowercase().as_str() {
        "jpg" | "jpeg" | "jpe" => "JPEG",
        "png" => "PNG",
        "gif" => "GIF",
        "webp" => "WebP",
        "bmp" => "BMP",
        "tiff" | "tif" => "TIFF",
        "ico" => "ICO",
        "avif" => "AVIF",
        "hdr" => "HDR",
        "jxl" => "JPEG XL",
        "ppm" => "PPM",
        "pgm" => "PGM",
        "pbm" => "PBM",
        "pnm" => "PNM",
        "qoi" => "QOI",
        "ff" | "farbfeld" => "Farbfeld",
        "svg" | "svgz" => "SVG",
        "heif" | "heic" => "HEIF",
        raw if raw_mime_type(raw).is_some() => "RAW",
        _ => return fl!("image-type-generic"),
    };
    fl!("image-type-format", format = format)
}

// reason: file sizes never approach 2^52 bytes, so f64 represents them exactly.
// reason: crop region is a non-negative, in-bounds pixel rectangle.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn crop_to_region(img: &DynamicImage, region: Rectangle) -> DynamicImage {
    img.crop_imm(
        region.x as u32,
        region.y as u32,
        region.width as u32,
        region.height as u32,
    )
}

// reason: normalized color channels are in 0.0..=1.0, so *255 lands in u8 range.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn text_color_rgba(c: Color) -> cosmic_text::Color {
    cosmic_text::Color::rgba(
        (c.r * 255.0) as u8,
        (c.g * 255.0) as u8,
        (c.b * 255.0) as u8,
        (c.a * 255.0) as u8,
    )
}

#[allow(clippy::cast_precision_loss)]
fn format_file_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * KB;
    const GB: f64 = 1024.0 * MB;

    let human_readable = if bytes as f64 >= GB {
        format!("{:.1} {}", bytes as f64 / GB, fl!("size-gb"))
    } else if bytes as f64 >= MB {
        format!("{:.1} {}", bytes as f64 / MB, fl!("size-mb"))
    } else if bytes as f64 >= KB {
        format!("{:.1} {}", bytes as f64 / KB, fl!("size-kb"))
    } else {
        format!("{bytes} {}", fl!("size-b"))
    };

    let grouped = format_number(bytes);
    let exact = fl!("size-bytes", bytes = grouped.as_str(), count = bytes);
    format!("{human_readable} {exact}")
}

fn format_number(num: u64) -> String {
    let s = num.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (idx, ch) in s.chars().enumerate() {
        if idx > 0 && (s.len() - idx).is_multiple_of(3) {
            result.push(',');
        }

        result.push(ch);
    }
    result
}

fn format_system_time(time: std::time::SystemTime) -> String {
    let date_time: chrono::DateTime<chrono::Local> = time.into();
    // The pattern comes from the locale: `to_string()` panics on a malformed one (and the
    // release profile aborts), while `write!` reports it as an error.
    let pattern = fl!("date-time-format");
    let mut formatted = String::new();
    if write!(formatted, "{}", date_time.format(&pattern)).is_err() {
        formatted = date_time.format("%a %d %b %Y %I:%M:%S %p %Z").to_string();
    }
    formatted
}

// reason: the write guard must live across the whole `.faces()` walk (its single
// use); clippy's auto-merge suggestion can't see through the borrow chain.
#[allow(clippy::significant_drop_tightening)]
fn load_font_families() -> Vec<&'static str> {
    // Scope the write guard so the font-system lock is released before sorting.
    let mut families: Vec<String> = {
        let mut font_sys = font_system().write().expect("Read font system");
        let unique: HashSet<String> = font_sys
            .raw()
            .db()
            .faces()
            .flat_map(|face| face.families.iter().map(|(name, _)| name.clone()))
            .collect();
        unique.into_iter().collect()
    };

    families.sort();

    families
        .into_iter()
        .map(|name| &*Box::leak(name.into_boxed_str()))
        .collect()
}

// Wallpaper functions

fn is_cosmic_desktop() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_uppercase().contains("WMDE"))
}

fn get_cosmic_outputs() -> Vec<String> {
    if let Ok(output) = std::process::Command::new("wmde-randr")
        .arg("list")
        .output()
        && output.status.success()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stripped = strip_ansi_codes(&stdout);
        stripped
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.contains("(enabled)") || line.contains("(disabled)") {
                    line.split_whitespace().next().map(String::from)
                } else {
                    None
                }
            })
            .collect()
    } else {
        Vec::new()
    }
}

fn strip_ansi_codes(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            result.push(c);
        }
    }

    result
}

#[cfg(not(target_os = "linux"))]
fn set_wallpaper_portable(path: &std::path::Path) -> Result<(), String> {
    wallpaper::set_from_path(
        path.to_str()
            .ok_or_else(|| "Invalid file path".to_string())?,
    )
    .map_err(|e| format!("Failed to set wallpaper: {e}"))
}

/// Override the base Icon appearance with the COSMIC selected-icon-button look when selected:
/// `@selected_color` background + `@accent_text_color` glyph/label. Non-selected = untouched Icon style.
fn apply_selected(mut base: button::Style, selected: bool, t: &Theme) -> button::Style {
    if selected {
        let cosmic = t.cosmic();
        base.background = Some(Background::Color(
            cosmic.icon_button.selected_state_color().into(),
        ));
        base.icon_color = Some(cosmic.accent_text_color().into());
        base.text_color = Some(cosmic.accent_text_color().into());
    }

    base
}

/// Class for a toggle icon button. Delegates to the base `Button::Icon` Catalog for each state
/// so hover/press/focus behave normally and layers the selected look on top.
fn tool_toggle_class(selected: bool) -> theme::Button {
    theme::Button::Custom {
        active: Box::new(move |f, t| {
            apply_selected(
                button::Catalog::active(t, f, selected, &theme::Button::Icon),
                selected,
                t,
            )
        }),
        hovered: Box::new(move |f, t| {
            apply_selected(
                button::Catalog::hovered(t, f, selected, &theme::Button::Icon),
                selected,
                t,
            )
        }),
        pressed: Box::new(move |f, t| {
            apply_selected(
                button::Catalog::pressed(t, f, selected, &theme::Button::Icon),
                selected,
                t,
            )
        }),
        disabled: Box::new(move |t| {
            apply_selected(
                button::Catalog::disabled(t, &theme::Button::Icon),
                selected,
                t,
            )
        }),
    }
}
