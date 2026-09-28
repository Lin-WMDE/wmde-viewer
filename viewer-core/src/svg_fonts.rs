// SPDX-License-Identifier: GPL-3.0-only

// WMDE: the fonts a drawing is set with. The default options carry no fonts, and a drawing lost
// all its text. The system's fonts are loaded once for the process and shared by every drawing.
//
// Two things follow fontconfig, as a browser on the same desktop does. fontdb names the generic
// families after fonts a Linux desktop seldom has (Times New Roman, Arial, Courier New); the
// stack's own Noto families stand in where they are installed. And a family a drawing names
// that is not installed - Helvetica from draw.io, Arial and Calibri from an office suite - goes
// the way fontconfig's own `<alias>` rules send it, to a metric twin that is installed or to a
// generic family, instead of usvg's last resort of the serif one.

use resvg::usvg::{
    self, FontFamily, FontStretch, FontStyle,
    fontdb::{self, Database},
};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, OnceLock},
};

/// How far an alias is followed to another: Calibri to Carlito to sans-serif is two steps.
const ALIAS_DEPTH: u8 = 3;

/// The options every drawing is read with, for showing it, its thumbnail and a wallpaper.
pub fn svg_options() -> usvg::Options<'static> {
    let fonts = fonts();
    usvg::Options {
        // Text with no family of its own takes the serif one, as a browser sets it.
        font_family: fonts
            .database
            .family_name(&fontdb::Family::Serif)
            .to_owned(),
        fontdb: Arc::clone(&fonts.database),
        font_resolver: usvg::FontResolver {
            select_font: Box::new(select_font),
            select_fallback: usvg::FontResolver::default_fallback_selector(),
        },
        ..usvg::Options::default()
    }
}

pub struct Fonts {
    database: Arc<Database>,
    /// Installed family names by their lowercase form.
    installed: HashMap<String, String>,
    /// What fontconfig puts in place of a family, by its lowercase name, in rule order.
    aliases: HashMap<String, Vec<String>>,
}

fn fonts() -> &'static Fonts {
    static FONTS: OnceLock<Fonts> = OnceLock::new();
    FONTS.get_or_init(|| {
        let mut database = Database::new();
        database.load_system_fonts();
        let installed = installed_families(&database);
        let has = |family: &str| installed.contains_key(&family.to_lowercase());
        if has("Noto Serif") {
            database.set_serif_family("Noto Serif");
        }
        if has("Noto Sans") {
            database.set_sans_serif_family("Noto Sans");
        }
        if has("Noto Sans Mono") {
            database.set_monospace_family("Noto Sans Mono");
        }
        Fonts {
            database: Arc::new(database),
            installed,
            aliases: fontconfig_aliases(),
        }
    })
}

fn installed_families(database: &Database) -> HashMap<String, String> {
    database
        .faces()
        .flat_map(|face| face.families.iter())
        .map(|(name, _)| (name.to_lowercase(), name.clone()))
        .collect()
}

/// The `<alias>` rules of fontconfig's configuration, read from where fontdb reads it.
fn fontconfig_aliases() -> HashMap<String, Vec<String>> {
    let mut config = fontconfig_parser::FontConfig::default();
    if let Some(file) = std::env::var_os("FONTCONFIG_FILE") {
        let _ = config.merge_config(Path::new(&file));
    } else {
        let user = dirs::config_dir().map(|dir| dir.join("fontconfig").join("fonts.conf"));
        if user.is_none_or(|user| config.merge_config(&user).is_err()) {
            let _ = config.merge_config(Path::new("/etc/fonts/local.conf"));
        }
        let _ = config.merge_config(Path::new("/etc/fonts/fonts.conf"));
    }
    aliases_of(config.aliases)
}

fn aliases_of(rules: Vec<fontconfig_parser::Alias>) -> HashMap<String, Vec<String>> {
    let mut aliases: HashMap<String, Vec<String>> = HashMap::new();
    for rule in rules {
        aliases
            .entry(rule.alias.to_lowercase())
            .or_default()
            .extend(
                rule.prefer
                    .into_iter()
                    .chain(rule.accept)
                    .chain(rule.default),
            );
    }
    aliases
}

impl Fonts {
    /// The family to look for in place of `name`: itself when installed, else the first of
    /// what fontconfig puts in its place that is installed or generic, `depth` aliases deep.
    fn resolve(&self, name: &str, depth: u8) -> Option<FontFamily> {
        if let Some(installed) = self.installed.get(&name.to_lowercase()) {
            return Some(FontFamily::Named(installed.clone()));
        }
        if depth == 0 {
            return None;
        }
        self.aliases
            .get(&name.to_lowercase())?
            .iter()
            .find_map(|candidate| generic(candidate).or_else(|| self.resolve(candidate, depth - 1)))
    }
}

fn generic(name: &str) -> Option<FontFamily> {
    Some(match name.to_lowercase().as_str() {
        "serif" => FontFamily::Serif,
        "sans-serif" | "sans serif" | "sans" => FontFamily::SansSerif,
        "monospace" | "mono" => FontFamily::Monospace,
        "cursive" => FontFamily::Cursive,
        "fantasy" => FontFamily::Fantasy,
        _ => return None,
    })
}

/// usvg's own selection, the named families put through `resolve` first.
fn select_font(font: &usvg::Font, database: &mut Arc<Database>) -> Option<fontdb::ID> {
    let fonts = fonts();
    let families: Vec<FontFamily> = font
        .families()
        .iter()
        .map(|family| match family {
            FontFamily::Named(name) => fonts
                .resolve(name, ALIAS_DEPTH)
                .unwrap_or_else(|| family.clone()),
            generic => generic.clone(),
        })
        .collect();
    let mut names: Vec<fontdb::Family> = families
        .iter()
        .map(|family| match family {
            FontFamily::Serif => fontdb::Family::Serif,
            FontFamily::SansSerif => fontdb::Family::SansSerif,
            FontFamily::Cursive => fontdb::Family::Cursive,
            FontFamily::Fantasy => fontdb::Family::Fantasy,
            FontFamily::Monospace => fontdb::Family::Monospace,
            FontFamily::Named(name) => fontdb::Family::Name(name),
        })
        .collect();
    names.push(fontdb::Family::Serif);
    database.query(&fontdb::Query {
        families: &names,
        weight: fontdb::Weight(font.weight()),
        stretch: match font.stretch() {
            FontStretch::UltraCondensed => fontdb::Stretch::UltraCondensed,
            FontStretch::ExtraCondensed => fontdb::Stretch::ExtraCondensed,
            FontStretch::Condensed => fontdb::Stretch::Condensed,
            FontStretch::SemiCondensed => fontdb::Stretch::SemiCondensed,
            FontStretch::Normal => fontdb::Stretch::Normal,
            FontStretch::SemiExpanded => fontdb::Stretch::SemiExpanded,
            FontStretch::Expanded => fontdb::Stretch::Expanded,
            FontStretch::ExtraExpanded => fontdb::Stretch::ExtraExpanded,
            FontStretch::UltraExpanded => fontdb::Stretch::UltraExpanded,
        },
        style: match font.style() {
            FontStyle::Normal => fontdb::Style::Normal,
            FontStyle::Italic => fontdb::Style::Italic,
            FontStyle::Oblique => fontdb::Style::Oblique,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fontconfig `<alias>`: the family, then its prefer, accept and default lists.
    type Rule<'a> = (&'a str, &'a [&'a str], &'a [&'a str], &'a [&'a str]);

    fn fonts(installed: &[&str], rules: &[Rule]) -> Fonts {
        Fonts {
            database: Arc::new(Database::new()),
            installed: installed
                .iter()
                .map(|name| (name.to_lowercase(), (*name).to_owned()))
                .collect(),
            aliases: aliases_of(
                rules
                    .iter()
                    .map(|(alias, prefer, accept, default)| {
                        let list = |names: &[&str]| names.iter().map(|&n| n.to_owned()).collect();
                        fontconfig_parser::Alias {
                            alias: (*alias).to_owned(),
                            prefer: list(prefer),
                            accept: list(accept),
                            default: list(default),
                        }
                    })
                    .collect(),
            ),
        }
    }

    #[test]
    fn an_installed_family_is_itself_whatever_its_case() {
        let fonts = fonts(&["Noto Sans"], &[]);
        assert_eq!(
            fonts.resolve("noto sans", ALIAS_DEPTH),
            Some(FontFamily::Named("Noto Sans".into()))
        );
    }

    #[test]
    fn a_missing_family_goes_where_fontconfig_sends_it() {
        // 30-metric-aliases and 45-latin, as they read on the VM.
        let rules: &[Rule] = &[
            ("Helvetica", &[], &["Liberation Sans"], &[]),
            ("Helvetica", &[], &[], &["sans-serif"]),
            ("Calibri", &[], &["Carlito"], &[]),
            ("Carlito", &[], &[], &["sans-serif"]),
            ("Courier New", &[], &[], &["monospace"]),
        ];
        let with_twin = fonts(&["Liberation Sans"], rules);
        assert_eq!(
            with_twin.resolve("Helvetica", ALIAS_DEPTH),
            Some(FontFamily::Named("Liberation Sans".into()))
        );
        let without = fonts(&["Noto Sans"], rules);
        assert_eq!(
            without.resolve("Helvetica", ALIAS_DEPTH),
            Some(FontFamily::SansSerif)
        );
        assert_eq!(
            without.resolve("Calibri", ALIAS_DEPTH),
            Some(FontFamily::SansSerif)
        );
        assert_eq!(
            without.resolve("Courier New", ALIAS_DEPTH),
            Some(FontFamily::Monospace)
        );
        assert_eq!(without.resolve("Unheard Of", ALIAS_DEPTH), None);
    }

    #[test]
    fn aliases_that_name_each_other_end() {
        let rules: &[Rule] = &[("A", &["B"], &[], &[]), ("B", &["A"], &[], &[])];
        assert_eq!(fonts(&[], rules).resolve("A", ALIAS_DEPTH), None);
    }
}
