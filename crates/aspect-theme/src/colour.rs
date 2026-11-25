//! Colours, the colour depth a terminal takes, and the nearest colour at a
//! shallower depth.

/// How many colours a stream renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Depth {
    /// The terminal's own 16 colours (`30`-`37`, `90`-`97`).
    Ansi16,
    /// The xterm 256-colour palette (`38;5;N`).
    Ansi256,
    /// 24-bit colour (`38;2;R;G;B`).
    TrueColor,
}

impl Depth {
    pub const ALL: [Depth; 3] = [Depth::TrueColor, Depth::Ansi256, Depth::Ansi16];

    /// The depth as a number: 24, 256 or 16.
    pub fn number(self) -> u32 {
        match self {
            Depth::Ansi16 => 16,
            Depth::Ansi256 => 256,
            Depth::TrueColor => 24,
        }
    }

    /// The depth a terminal takes, from `COLORTERM` and `TERM`: 24-bit when
    /// `COLORTERM` says truecolor, 16 for a `TERM` that only does 16, else
    /// 256. CI hosts set neither, so they get 256.
    pub fn detect(colorterm: Option<&str>, term: Option<&str>) -> Depth {
        if matches!(colorterm, Some("truecolor" | "24bit")) {
            return Depth::TrueColor;
        }
        let Some(term) = term else {
            return Depth::Ansi256;
        };
        if term.contains("256color") {
            return Depth::Ansi256;
        }
        if term.contains("direct") || term.contains("truecolor") {
            return Depth::TrueColor;
        }
        let sixteen = matches!(
            term,
            "linux"
                | "ansi"
                | "cons25"
                | "cygwin"
                | "xterm"
                | "xterm-color"
                | "xterm-16color"
                | "screen"
                | "rxvt"
                | "putty"
        ) || term.starts_with("vt");
        if sixteen {
            Depth::Ansi16
        } else {
            Depth::Ansi256
        }
    }
}

/// A colour as written in an SGR sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Colour {
    /// One of the terminal's 16, 0-15.
    Ansi(u8),
    /// An entry of the 256-colour palette.
    Index(u8),
    Rgb(u8, u8, u8),
}

impl Colour {
    /// The SGR parameters that set this colour as the foreground, or with
    /// `background` as the background.
    pub fn params(self, background: bool) -> String {
        match self {
            Colour::Ansi(n) => {
                let base = match (background, n < 8) {
                    (false, true) => 30,
                    (false, false) => 90 - 8,
                    (true, true) => 40,
                    (true, false) => 100 - 8,
                };
                (base + u32::from(n)).to_string()
            }
            Colour::Index(i) => format!("{};5;{i}", if background { 48 } else { 38 }),
            Colour::Rgb(r, g, b) => format!("{};2;{r};{g};{b}", if background { 48 } else { 38 }),
        }
    }

    /// The RGB this colour draws as, ANSI colours as xterm draws them.
    pub fn rgb(self) -> [u8; 3] {
        match self {
            Colour::Ansi(n) => XTERM16[usize::from(n.min(15))],
            Colour::Index(i) => rgb256(i),
            Colour::Rgb(r, g, b) => [r, g, b],
        }
    }
}

/// The 16 ANSI colour names, at their indices.
pub const ANSI_NAMES: [&str; 16] = [
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "white",
    "bright-black",
    "bright-red",
    "bright-green",
    "bright-yellow",
    "bright-blue",
    "bright-magenta",
    "bright-cyan",
    "bright-white",
];

/// The index of an ANSI colour name.
pub fn ansi_index(name: &str) -> Option<u8> {
    ANSI_NAMES.iter().position(|n| *n == name).map(|i| i as u8)
}

const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];

const XTERM16: [[u8; 3]; 16] = [
    [0, 0, 0],
    [205, 0, 0],
    [0, 205, 0],
    [205, 205, 0],
    [0, 0, 238],
    [205, 0, 205],
    [0, 205, 205],
    [229, 229, 229],
    [127, 127, 127],
    [255, 0, 0],
    [0, 255, 0],
    [255, 255, 0],
    [92, 92, 255],
    [255, 0, 255],
    [0, 255, 255],
    [255, 255, 255],
];

/// The RGB of a 256-palette entry, as xterm draws it.
pub fn rgb256(i: u8) -> [u8; 3] {
    match i {
        0..=15 => XTERM16[usize::from(i)],
        232..=255 => {
            let g = 8 + 10 * (i - 232);
            [g, g, g]
        }
        _ => {
            let i = usize::from(i - 16);
            [LEVELS[i / 36], LEVELS[(i / 6) % 6], LEVELS[i % 6]]
        }
    }
}

fn distance(a: [u8; 3], b: [u8; 3]) -> u32 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (i32::from(*x) - i32::from(y)).pow(2) as u32)
        .sum()
}

/// The nearest 256-palette entry: the nearer of the colour cube's and the
/// grey ramp's.
pub fn nearest256(c: [u8; 3]) -> u8 {
    let level = |v: u8| {
        (0..6)
            .min_by_key(|&i| (i32::from(LEVELS[i]) - i32::from(v)).abs())
            .expect("six levels") as u8
    };
    let cube = 16 + 36 * level(c[0]) + 6 * level(c[1]) + level(c[2]);
    let mean = (u32::from(c[0]) + u32::from(c[1]) + u32::from(c[2])) / 3;
    let gi = (mean.saturating_sub(8) / 10).min(23) as u8;
    let g = 8 + 10 * gi;
    if distance([g, g, g], c) < distance(rgb256(cube), c) {
        232 + gi
    } else {
        cube
    }
}

/// The nearest of the terminal's 16, black and the two whites left out: a
/// colour made to be read on the terminal's background never lands on it.
pub fn nearest16(c: [u8; 3]) -> u8 {
    (1..16u8)
        .filter(|i| !matches!(i, 7 | 15))
        .min_by_key(|&i| distance(XTERM16[usize::from(i)], c))
        .expect("candidates")
}

/// `#rrggbb` or `#rgb`.
pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let hex = s.strip_prefix('#')?;
    let digits: Vec<u8> = match hex.len() {
        3 => hex.chars().flat_map(|c| [c, c]).map(|c| c as u8).collect(),
        6 => hex.bytes().collect(),
        _ => return None,
    };
    let byte = |i: usize| {
        std::str::from_utf8(&digits[i..i + 2])
            .ok()
            .and_then(|s| u8::from_str_radix(s, 16).ok())
    };
    Some([byte(0)?, byte(2)?, byte(4)?])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_follows_colorterm_then_term() {
        assert_eq!(
            Depth::detect(Some("truecolor"), Some("xterm")),
            Depth::TrueColor
        );
        assert_eq!(Depth::detect(Some("24bit"), None), Depth::TrueColor);
        assert_eq!(Depth::detect(None, None), Depth::Ansi256, "CI");
        assert_eq!(Depth::detect(None, Some("xterm-256color")), Depth::Ansi256);
        assert_eq!(Depth::detect(None, Some("tmux-256color")), Depth::Ansi256);
        assert_eq!(Depth::detect(None, Some("xterm-direct")), Depth::TrueColor);
        for sixteen in ["linux", "vt100", "vt220", "ansi", "xterm", "screen"] {
            assert_eq!(
                Depth::detect(None, Some(sixteen)),
                Depth::Ansi16,
                "{sixteen}"
            );
        }
        assert_eq!(Depth::detect(None, Some("xterm-kitty")), Depth::Ansi256);
    }

    #[test]
    fn nearest_colours_match_the_design() {
        assert_eq!(nearest256([0x6a, 0xa6, 0xff]), 75);
        assert_eq!(nearest256([0x97, 0x9e, 0xa8]), 247);
        assert_eq!(nearest256([0x65, 0x6c, 0x76]), 242);
        assert_eq!(nearest256([0xf7, 0x5f, 0x69]), 203);
        assert_eq!(nearest256([0, 0, 0]), 16);
        assert_eq!(nearest16([0x6a, 0xa6, 0xff]), 12);
        assert_ne!(nearest16([0, 0, 0]), 0);
        assert_ne!(nearest16([255, 255, 255]), 15);
    }

    #[test]
    fn params_per_kind() {
        assert_eq!(Colour::Ansi(1).params(false), "31");
        assert_eq!(Colour::Ansi(9).params(false), "91");
        assert_eq!(Colour::Ansi(6).params(true), "46");
        assert_eq!(Colour::Ansi(14).params(true), "106");
        assert_eq!(Colour::Index(75).params(false), "38;5;75");
        assert_eq!(Colour::Rgb(1, 2, 3).params(true), "48;2;1;2;3");
    }

    #[test]
    fn hex_forms() {
        assert_eq!(parse_hex("#6aa6ff"), Some([0x6a, 0xa6, 0xff]));
        assert_eq!(parse_hex("#6AF"), Some([0x66, 0xaa, 0xff]));
        assert_eq!(parse_hex("6aa6ff"), None);
        assert_eq!(parse_hex("#6aa6f"), None);
        assert_eq!(parse_hex("#gggggg"), None);
    }
}
