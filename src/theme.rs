// Portions adapted from Sofka's src/theme.rs.
// Copyright 2026 Nikola Milojević. Licensed under Apache-2.0.
// Translated and modified for xpdelve in 2026.
// See NOTICE for full provenance.
#![allow(dead_code)]

use std::collections::HashMap;

use anyhow::{Result, bail};
use ratatui::style::{Color, Modifier, Style};
use terminal_colorsaurus::{QueryOptions, ThemeMode};

use crate::config::{ColorMode, SkinConfig};
use crate::model::Health;

pub const BUILTIN_NAMES: &[&str] = &[
    "catppuccin-mocha",
    "catppuccin-latte",
    "catppuccin-frappe",
    "catppuccin-macchiato",
    "gruvbox-dark",
    "gruvbox-light",
    "nord",
    "dracula",
    "solarized-dark",
    "solarized-light",
    "tokyo-night",
    "one-dark",
    "rose-pine",
    "rose-pine-dawn",
    "monokai",
    "flexoki-dark",
    "flexoki-light",
];

const SWATCH_NAMES: &[&str] = &[
    "rosewater",
    "flamingo",
    "pink",
    "mauve",
    "red",
    "maroon",
    "peach",
    "yellow",
    "green",
    "teal",
    "sky",
    "sapphire",
    "blue",
    "lavender",
    "text",
    "subtext1",
    "subtext0",
    "overlay1",
    "overlay0",
    "surface2",
    "surface1",
    "surface0",
    "base",
    "mantle",
    "crust",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Palette {
    pub rosewater: Color,
    pub flamingo: Color,
    pub pink: Color,
    pub mauve: Color,
    pub red: Color,
    pub maroon: Color,
    pub peach: Color,
    pub yellow: Color,
    pub green: Color,
    pub teal: Color,
    pub sky: Color,
    pub sapphire: Color,
    pub blue: Color,
    pub lavender: Color,
    pub text: Color,
    pub subtext1: Color,
    pub subtext0: Color,
    pub overlay1: Color,
    pub overlay0: Color,
    pub surface2: Color,
    pub surface1: Color,
    pub surface0: Color,
    pub base: Color,
    pub mantle: Color,
    pub crust: Color,
}

impl Palette {
    fn from_hexes(values: &[&str; 25]) -> Self {
        let color = |index| parse_hex(values[index]).expect("built-in colors are valid");
        Self {
            rosewater: color(0),
            flamingo: color(1),
            pink: color(2),
            mauve: color(3),
            red: color(4),
            maroon: color(5),
            peach: color(6),
            yellow: color(7),
            green: color(8),
            teal: color(9),
            sky: color(10),
            sapphire: color(11),
            blue: color(12),
            lavender: color(13),
            text: color(14),
            subtext1: color(15),
            subtext0: color(16),
            overlay1: color(17),
            overlay0: color(18),
            surface2: color(19),
            surface1: color(20),
            surface0: color(21),
            base: color(22),
            mantle: color(23),
            crust: color(24),
        }
    }

    fn set(&mut self, name: &str, color: Color) -> bool {
        let slot = match name {
            "rosewater" => &mut self.rosewater,
            "flamingo" => &mut self.flamingo,
            "pink" => &mut self.pink,
            "mauve" => &mut self.mauve,
            "red" => &mut self.red,
            "maroon" => &mut self.maroon,
            "peach" => &mut self.peach,
            "yellow" => &mut self.yellow,
            "green" => &mut self.green,
            "teal" => &mut self.teal,
            "sky" => &mut self.sky,
            "sapphire" => &mut self.sapphire,
            "blue" => &mut self.blue,
            "lavender" => &mut self.lavender,
            "text" => &mut self.text,
            "subtext1" => &mut self.subtext1,
            "subtext0" => &mut self.subtext0,
            "overlay1" => &mut self.overlay1,
            "overlay0" => &mut self.overlay0,
            "surface2" => &mut self.surface2,
            "surface1" => &mut self.surface1,
            "surface0" => &mut self.surface0,
            "base" => &mut self.base,
            "mantle" => &mut self.mantle,
            "crust" => &mut self.crust,
            _ => return false,
        };
        *slot = color;
        true
    }
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub palette: Palette,
    pub configured_name: Option<String>,
    pub resolved_name: String,
    pub colors_enabled: bool,
}

impl Theme {
    pub fn resolve(skin: &SkinConfig, mode: ColorMode) -> Result<Self> {
        let colors_enabled = colors_enabled(mode, std::env::var_os("NO_COLOR").is_some());
        let resolved_name = skin
            .name
            .as_deref()
            .map(canonical_name)
            .transpose()?
            .map_or_else(auto_skin_name, str::to_owned);
        Self::resolve_named(skin, colors_enabled, &resolved_name)
    }

    pub fn resolve_for_mode(
        skin: &SkinConfig,
        mode: ColorMode,
        no_color: bool,
        terminal_mode: Option<ThemeMode>,
    ) -> Result<Self> {
        let resolved_name = skin
            .name
            .as_deref()
            .map(canonical_name)
            .transpose()?
            .map_or_else(
                || match terminal_mode {
                    Some(ThemeMode::Light) => "catppuccin-latte".to_owned(),
                    _ => "catppuccin-mocha".to_owned(),
                },
                str::to_owned,
            );
        Self::resolve_named(skin, colors_enabled(mode, no_color), &resolved_name)
    }

    fn resolve_named(skin: &SkinConfig, colors_enabled: bool, resolved_name: &str) -> Result<Self> {
        let mut palette = builtin(resolved_name)
            .ok_or_else(|| anyhow::anyhow!("unknown skin {resolved_name:?}"))?;
        apply_overrides(&mut palette, &skin.colors)?;
        Ok(Self {
            palette,
            configured_name: skin.name.clone(),
            resolved_name: resolved_name.to_owned(),
            colors_enabled,
        })
    }

    pub fn title(&self) -> Style {
        self.fg(self.palette.teal).bold()
    }

    pub fn border(&self) -> Style {
        self.fg(self.palette.lavender)
    }

    pub fn selected_row(&self) -> Style {
        if self.colors_enabled {
            Style::default()
                .fg(self.palette.base)
                .bg(self.palette.lavender)
                .bold()
        } else {
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        }
    }

    pub fn danger(&self) -> Style {
        self.fg(self.palette.red).bold()
    }

    pub fn subtle(&self) -> Style {
        if self.colors_enabled {
            Style::default().fg(self.palette.overlay1)
        } else {
            Style::default().add_modifier(Modifier::DIM)
        }
    }

    pub fn warning(&self) -> Style {
        self.fg(self.palette.yellow)
    }

    pub fn selected_option(&self) -> Style {
        if self.colors_enabled {
            Style::default().fg(self.palette.yellow).bold()
        } else {
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        }
    }

    pub fn activity(&self) -> Style {
        self.fg(self.palette.sky)
    }

    pub fn health(&self, health: Health) -> Style {
        match health {
            Health::Healthy => self.fg(self.palette.green),
            Health::Warning => self.fg(self.palette.yellow),
            Health::Unhealthy => self.fg(self.palette.red),
            Health::Unknown => Style::default(),
        }
    }

    pub fn fg(&self, color: Color) -> Style {
        if self.colors_enabled {
            Style::default().fg(color)
        } else {
            Style::default()
        }
    }

    pub fn search(&self, base: Style) -> Style {
        if self.colors_enabled {
            base.bg(self.palette.yellow).fg(self.palette.base).bold()
        } else {
            base.add_modifier(Modifier::REVERSED | Modifier::BOLD)
        }
    }
}

pub fn colors_enabled(mode: ColorMode, no_color_present: bool) -> bool {
    match mode {
        ColorMode::Always => true,
        ColorMode::Never => false,
        ColorMode::Auto => !no_color_present,
    }
}

pub fn validate_skin(skin: &SkinConfig) -> Result<()> {
    if let Some(name) = skin.name.as_deref() {
        canonical_name(name)?;
    }
    let mut palette = builtin("catppuccin-mocha").expect("mocha is built in");
    apply_overrides(&mut palette, &skin.colors)
}

fn auto_skin_name() -> String {
    match terminal_colorsaurus::theme_mode(QueryOptions::default()).ok() {
        Some(ThemeMode::Light) => "catppuccin-latte".to_owned(),
        _ => "catppuccin-mocha".to_owned(),
    }
}

fn canonical_name(name: &str) -> Result<&'static str> {
    let normalized = name.trim().to_ascii_lowercase();
    let canonical = match normalized.as_str() {
        "mocha" => "catppuccin-mocha",
        "latte" => "catppuccin-latte",
        "frappe" => "catppuccin-frappe",
        "macchiato" => "catppuccin-macchiato",
        "gruvbox" => "gruvbox-dark",
        "solarized" => "solarized-dark",
        "tokyonight" => "tokyo-night",
        "onedark" => "one-dark",
        "rosepine" => "rose-pine",
        "rosepinedawn" => "rose-pine-dawn",
        "flexoki" => "flexoki-dark",
        value if BUILTIN_NAMES.contains(&value) => {
            return Ok(BUILTIN_NAMES
                .iter()
                .copied()
                .find(|candidate| *candidate == value)
                .expect("checked above"));
        }
        _ => bail!(
            "unknown skin {name:?}; expected one of {}",
            BUILTIN_NAMES.join(", ")
        ),
    };
    Ok(canonical)
}

fn apply_overrides(palette: &mut Palette, colors: &HashMap<String, String>) -> Result<()> {
    for (name, value) in colors {
        let name = name.trim().to_ascii_lowercase();
        if !SWATCH_NAMES.contains(&name.as_str()) {
            bail!("unknown skin color {name:?}");
        }
        let color = parse_hex(value)
            .ok_or_else(|| anyhow::anyhow!("invalid color {value:?} for skin.colors.{name}"))?;
        debug_assert!(palette.set(&name, color));
    }
    Ok(())
}

fn parse_hex(value: &str) -> Option<Color> {
    let value = value.trim().trim_start_matches('#');
    if value.len() != 6 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some(Color::Rgb(
        u8::from_str_radix(&value[0..2], 16).ok()?,
        u8::from_str_radix(&value[2..4], 16).ok()?,
        u8::from_str_radix(&value[4..6], 16).ok()?,
    ))
}

pub fn builtin(name: &str) -> Option<Palette> {
    let values = match name.trim().to_ascii_lowercase().as_str() {
        "catppuccin-mocha" | "mocha" => [
            "#f5e0dc", "#f2cdcd", "#f5c2e7", "#cba6f7", "#f38ba8", "#eba0ac", "#fab387", "#f9e2af",
            "#a6e3a1", "#94e2d5", "#89dceb", "#74c7ec", "#89b4fa", "#b4befe", "#cdd6f4", "#bac2de",
            "#a6adc8", "#7f849c", "#6c7086", "#585b70", "#45475a", "#313244", "#1e1e2e", "#181825",
            "#11111b",
        ],
        "catppuccin-latte" | "latte" => [
            "#dc8a78", "#dd7878", "#ea76cb", "#8839ef", "#d20f39", "#e64553", "#fe640b", "#df8e1d",
            "#40a02b", "#179299", "#04a5e5", "#209fb5", "#1e66f5", "#7287fd", "#4c4f69", "#5c5f77",
            "#6c6f85", "#8c8fa1", "#9ca0b0", "#acb0be", "#bcc0cc", "#ccd0da", "#eff1f5", "#e6e9ef",
            "#dce0e8",
        ],
        "catppuccin-frappe" | "frappe" => [
            "#f2d5cf", "#eebebe", "#f4b8e4", "#ca9ee6", "#e78284", "#ea999c", "#ef9f76", "#e5c890",
            "#a6d189", "#81c8be", "#99d1db", "#85c1dc", "#8caaee", "#babbf1", "#c6d0f5", "#b5bfe2",
            "#a5adce", "#949cbb", "#838ba7", "#626880", "#51576d", "#414559", "#303446", "#292c3c",
            "#232634",
        ],
        "catppuccin-macchiato" | "macchiato" => [
            "#f4dbd6", "#f0c6c6", "#f5bde6", "#c6a0f6", "#ed8796", "#ee99a0", "#f5a97f", "#eed49f",
            "#a6da95", "#8bd5ca", "#91d7e3", "#7dc4e4", "#8aadf4", "#b7bdf8", "#cad3f5", "#b8c0e0",
            "#a5adcb", "#8087a2", "#6e738d", "#5b6078", "#494d64", "#363a4f", "#24273a", "#1e2030",
            "#181926",
        ],
        "gruvbox" | "gruvbox-dark" => [
            "#ebdbb2", "#d5c4a1", "#d3869b", "#d3869b", "#fb4934", "#cc241d", "#fe8019", "#fabd2f",
            "#b8bb26", "#8ec07c", "#83a598", "#458588", "#83a598", "#d3869b", "#ebdbb2", "#d5c4a1",
            "#bdae93", "#a89984", "#928374", "#665c54", "#504945", "#3c3836", "#282828", "#1d2021",
            "#1d2021",
        ],
        "gruvbox-light" => [
            "#ebdbb2", "#d5c4a1", "#b16286", "#b16286", "#cc241d", "#9d0006", "#d65d0e", "#d79921",
            "#98971a", "#689d6a", "#458588", "#076678", "#458588", "#b16286", "#282828", "#3c3836",
            "#504945", "#665c54", "#7c6f64", "#928374", "#a89984", "#bdae93", "#fbf1c7", "#ebdbb2",
            "#d5c4a1",
        ],
        "nord" => [
            "#d8dee9", "#e5e9f0", "#b48ead", "#b48ead", "#bf616a", "#bf616a", "#d08770", "#ebcb8b",
            "#a3be8c", "#8fbcbb", "#88c0d0", "#81a1c1", "#5e81ac", "#b48ead", "#eceff4", "#e5e9f0",
            "#d8dee9", "#616e88", "#4c566a", "#434c5e", "#3b4252", "#333a47", "#2e3440", "#2b303b",
            "#242933",
        ],
        "dracula" => [
            "#f8f8f2", "#ffb86c", "#ff79c6", "#bd93f9", "#ff5555", "#ff5555", "#ffb86c", "#f1fa8c",
            "#50fa7b", "#8be9fd", "#8be9fd", "#62d6e8", "#6272a4", "#bd93f9", "#f8f8f2", "#d8d8d2",
            "#b8b8b2", "#6272a4", "#565761", "#44475a", "#3a3c4e", "#343746", "#282a36", "#21222c",
            "#191a21",
        ],
        "solarized-dark" | "solarized" => [
            "#d33682", "#d33682", "#d33682", "#6c71c4", "#dc322f", "#dc322f", "#cb4b16", "#b58900",
            "#859900", "#2aa198", "#2aa198", "#268bd2", "#268bd2", "#6c71c4", "#93a1a1", "#839496",
            "#657b83", "#586e75", "#586e75", "#586e75", "#073642", "#073642", "#002b36", "#002b36",
            "#001e26",
        ],
        "solarized-light" => [
            "#d33682", "#d33682", "#d33682", "#6c71c4", "#dc322f", "#dc322f", "#cb4b16", "#b58900",
            "#859900", "#2aa198", "#2aa198", "#268bd2", "#268bd2", "#6c71c4", "#002b36", "#073642",
            "#586e75", "#657b83", "#839496", "#93a1a1", "#eee8d5", "#eee8d5", "#fdf6e3", "#eee8d5",
            "#eee8d5",
        ],
        "tokyo-night" | "tokyonight" => [
            "#f7768e", "#f7768e", "#bb9af7", "#9d7cd8", "#f7768e", "#db4b4b", "#ff9e64", "#e0af68",
            "#9ece6a", "#73daca", "#7dcfff", "#0db9d7", "#7aa2f7", "#9d7cd8", "#c0caf5", "#a9b1d6",
            "#545c7e", "#545c7e", "#3b4261", "#3b4261", "#292e42", "#292e42", "#1a1b26", "#16161e",
            "#13131a",
        ],
        "one-dark" | "onedark" => [
            "#e06c75", "#e06c75", "#c678dd", "#c678dd", "#e06c75", "#be5046", "#d19a66", "#e5c07b",
            "#98c379", "#56b6c2", "#56b6c2", "#61afef", "#61afef", "#c678dd", "#abb2bf", "#828997",
            "#5c6370", "#5c6370", "#4b5263", "#4b5263", "#3b4048", "#323842", "#282c34", "#21252b",
            "#1b1d23",
        ],
        "rose-pine" | "rosepine" => [
            "#ebbcba", "#ebbcba", "#ebbcba", "#c4a7e7", "#eb6f92", "#eb6f92", "#f6c177", "#f6c177",
            "#31748f", "#31748f", "#9ccfd8", "#9ccfd8", "#9ccfd8", "#c4a7e7", "#e0def4", "#908caa",
            "#6e6a86", "#6e6a86", "#524f67", "#524f67", "#403d52", "#26233a", "#191724", "#191724",
            "#14121d",
        ],
        "rose-pine-dawn" | "rosepinedawn" => [
            "#d7827e", "#d7827e", "#d7827e", "#907aa9", "#b4637a", "#b4637a", "#ea9d34", "#ea9d34",
            "#286983", "#286983", "#56949f", "#56949f", "#56949f", "#907aa9", "#575279", "#797593",
            "#9893a5", "#9893a5", "#cecacd", "#cecacd", "#dfdad9", "#f2e9e1", "#faf4ed", "#faf4ed",
            "#f4ede8",
        ],
        "monokai" => [
            "#f92672", "#f92672", "#f92672", "#ae81ff", "#f92672", "#f92672", "#fd971f", "#e6db74",
            "#a6e22e", "#66d9ef", "#66d9ef", "#66d9ef", "#66d9ef", "#ae81ff", "#f8f8f2", "#cfcfc2",
            "#75715e", "#75715e", "#49483e", "#49483e", "#3e3d32", "#3e3d32", "#272822", "#23241f",
            "#1e1f1c",
        ],
        "flexoki-dark" | "flexoki" => [
            "#E47DA8", "#CE5D97", "#CE5D97", "#8B7EC8", "#D14D41", "#AF3029", "#DA702C", "#D0A215",
            "#879A39", "#3AA99F", "#5ABDAC", "#66A0C8", "#4385BE", "#A699D0", "#CECDC3", "#B7B5AC",
            "#878580", "#6F6E69", "#575653", "#403E3C", "#343331", "#282726", "#100F0F", "#1C1B1A",
            "#100F0F",
        ],
        "flexoki-light" => [
            "#CE5D97", "#A02F6F", "#A02F6F", "#5E409D", "#AF3029", "#C03E35", "#BC5215", "#AD8301",
            "#66800B", "#24837B", "#2F968D", "#3171B2", "#205EA6", "#735EB5", "#100F0F", "#575653",
            "#6F6E69", "#878580", "#9F9D96", "#B7B5AC", "#CECDC3", "#DAD8CE", "#FFFCF0", "#F2F0E5",
            "#E6E4D9",
        ],
        _ => return None,
    };
    Some(Palette::from_hexes(&values))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_builtin_skins_resolve() {
        for name in BUILTIN_NAMES {
            assert!(builtin(name).is_some(), "missing {name}");
        }
    }

    #[test]
    fn auto_mode_selects_light_or_dark_palette() {
        let skin = SkinConfig::default();
        let light =
            Theme::resolve_for_mode(&skin, ColorMode::Auto, false, Some(ThemeMode::Light)).unwrap();
        let dark =
            Theme::resolve_for_mode(&skin, ColorMode::Auto, false, Some(ThemeMode::Dark)).unwrap();
        assert_eq!(light.resolved_name, "catppuccin-latte");
        assert_eq!(dark.resolved_name, "catppuccin-mocha");
    }

    #[test]
    fn strict_overrides_reject_bad_input() {
        let mut skin = SkinConfig::default();
        skin.colors.insert("unknown".into(), "#ffffff".into());
        assert!(validate_skin(&skin).is_err());
        skin.colors.clear();
        skin.colors.insert("red".into(), "not-a-color".into());
        assert!(validate_skin(&skin).is_err());
    }

    #[test]
    fn no_color_policy_is_preserved() {
        assert!(!colors_enabled(ColorMode::Auto, true));
        assert!(colors_enabled(ColorMode::Always, true));
        assert!(!colors_enabled(ColorMode::Never, false));
    }

    #[test]
    fn mocha_keeps_the_established_xpdelve_roles() {
        let theme = Theme::resolve_for_mode(
            &SkinConfig {
                name: Some("catppuccin-mocha".into()),
                colors: HashMap::new(),
            },
            ColorMode::Always,
            false,
            None,
        )
        .unwrap();
        assert_eq!(theme.palette.lavender, Color::Rgb(180, 190, 254));
        assert_eq!(theme.palette.teal, Color::Rgb(148, 226, 213));
        assert_eq!(theme.palette.red, Color::Rgb(243, 139, 168));
        assert_eq!(theme.palette.overlay1, Color::Rgb(127, 132, 156));
        assert_eq!(theme.palette.yellow, Color::Rgb(249, 226, 175));
        assert_eq!(theme.palette.base, Color::Rgb(30, 30, 46));
    }

    #[test]
    fn overrides_change_semantic_styles() {
        let mut colors = HashMap::new();
        colors.insert("lavender".into(), "#123456".into());
        let theme = Theme::resolve_for_mode(
            &SkinConfig {
                name: Some("catppuccin-mocha".into()),
                colors,
            },
            ColorMode::Always,
            false,
            None,
        )
        .unwrap();
        assert_eq!(theme.border().fg, Some(Color::Rgb(0x12, 0x34, 0x56)));
        assert_eq!(theme.selected_row().bg, Some(Color::Rgb(0x12, 0x34, 0x56)));
    }
}
