//! Default Windows virtual-key → sound mapping (ported from keycode-to-sound.cjs).
//!
//! Sound ids: `&.x` voice syllable, `%.N` instrument note (MIDI), `sfx.name`, `#cmd` special.

pub struct KeyDef {
    pub key: &'static str,
    pub sound: &'static str,
    pub shift: Option<&'static str>,
    pub ctrl: &'static str,
}

const fn k(key: &'static str, sound: &'static str) -> KeyDef {
    KeyDef { key, sound, shift: None, ctrl: "" }
}
const fn ks(key: &'static str, sound: &'static str, ctrl: &'static str, shift: &'static str) -> KeyDef {
    KeyDef { key, sound, shift: Some(shift), ctrl }
}
const fn kc(key: &'static str, sound: &'static str, ctrl: &'static str) -> KeyDef {
    KeyDef { key, sound, shift: None, ctrl }
}

impl KeyDef {
    /// Shift falls back to the plain sound when not specified.
    pub fn shift_sound(&self) -> &'static str {
        self.shift.unwrap_or(self.sound)
    }
}

pub fn lookup(vk: u32) -> Option<&'static KeyDef> {
    KEYS.iter().find(|(c, _)| *c == vk).map(|(_, d)| d)
}

/// Virtual-key code for a key name as stored in hotkeys ("m", "F5", "Space", ...).
pub fn vk_for_name(name: &str) -> Option<u32> {
    KEYS.iter().find(|(_, d)| d.key.eq_ignore_ascii_case(name)).map(|(c, _)| *c)
}

/// Every (virtual key, definition) pair in table order.
pub fn all() -> impl Iterator<Item = (u32, &'static KeyDef)> {
    KEYS.iter().map(|(c, d)| (*c, d))
}

static KEYS: &[(u32, KeyDef)] = &[
    // Numbers
    (49, ks("1", "%.60", "%.61", "&.gwah")),
    (50, ks("2", "%.62", "%.63", "sfx.at")),
    (51, ks("3", "%.64", "%.65", "sfx.pound")),
    (52, ks("4", "%.65", "%.66", "sfx.dollar")),
    (53, ks("5", "%.67", "%.68", "sfx.percent")),
    (54, ks("6", "%.69", "%.70", "sfx.caret")),
    (55, ks("7", "%.71", "%.72", "sfx.ampersand")),
    (56, ks("8", "%.72", "%.73", "sfx.asterisk")),
    (57, ks("9", "%.74", "%.75", "sfx.parenthesis_open")),
    (48, ks("0", "%.76", "%.77", "sfx.parenthesis_closed")),
    // Letters
    (65, kc("a", "&.a", "sfx.parenthesis_open")),
    (66, k("b", "&.b")),
    (67, kc("c", "&.c", "sfx.slash_forward")),
    (68, k("d", "&.d")),
    (69, k("e", "&.e")),
    (70, k("f", "&.f")),
    (71, k("g", "&.g")),
    (72, k("h", "&.h")),
    (73, k("i", "&.i")),
    (74, k("j", "&.j")),
    (75, k("k", "&.k")),
    (76, k("l", "&.l")),
    (77, k("m", "&.m")),
    (78, k("n", "&.n")),
    (79, k("o", "&.o")),
    (80, k("p", "&.p")),
    (81, k("q", "&.q")),
    (82, k("r", "&.r")),
    (83, kc("s", "&.s", "sfx.asterisk")),
    (84, k("t", "&.t")),
    (85, k("u", "&.u")),
    (86, kc("v", "&.v", "sfx.pound")),
    (87, k("w", "&.w")),
    (88, kc("x", "&.x", "sfx.at")),
    (89, kc("y", "&.y", "sfx.ampersand")),
    (90, kc("z", "&.z", "sfx.ampersand")),
    // Function keys
    (112, k("F1", "")), (113, k("F2", "")), (114, k("F3", "")), (115, k("F4", "")),
    (116, k("F5", "")), (117, k("F6", "")), (118, k("F7", "")), (119, k("F8", "")),
    (120, k("F9", "")), (121, k("F10", "")), (122, k("F11", "")), (123, k("F12", "")),
    (124, k("F13", "")), (125, k("F14", "")), (126, k("F15", "")),
    // Special keys and characters
    (27, k("Esc", "sfx.enter")),
    (9, k("Tab", "sfx.tab")),
    (20, k("Caps Lock", "")),
    (192, ks("`", "%.55", "%.56", "sfx.tilde")),
    (189, ks("-", "%.77", "%.78", "sfx.default")),
    (187, ks("=", "%.79", "%.80", "sfx.default")),
    (219, ks("[", "sfx.bracket_open", "", "sfx.brace_open")),
    (221, ks("]", "sfx.bracket_closed", "", "sfx.brace_closed")),
    (220, ks("\\", "sfx.slash_back", "", "sfx.default")),
    (191, ks("/", "sfx.slash_forward", "", "&.deska")),
    (186, ks(";", "sfx.default", "", "sfx.default")),
    (222, ks("'", "sfx.default", "", "sfx.default")),
    (188, ks(",", "sfx.default", "", "sfx.default")),
    (190, ks(".", "sfx.default", "", "sfx.default")),
    (13, k("Enter", "sfx.enter")),
    (8, kc("Backspace", "sfx.backspace", "sfx.at")),
    (32, k("Space", "")),
    // Navigation
    (45, k("Insert", "")),
    (46, k("Delete", "sfx.at")),
    (36, k("Home", "")),
    (35, k("End", "")),
    (33, k("PageUp", "")),
    (34, k("PageDown", "")),
    (38, k("Up", "sfx.arrow_up")),
    (37, k("Left", "sfx.arrow_left")),
    (39, k("Right", "sfx.arrow_right")),
    (40, k("Down", "sfx.arrow_down")),
    // Modifiers
    (160, k("Shift", "")), (161, k("Shift", "")),
    (162, k("Ctrl", "")), (163, k("Ctrl", "")),
    (164, k("Alt", "")), (165, k("Alt", "")),
    (91, k("Meta", "")), (92, k("Meta", "")),
    (93, k("Menu", "")),
    // Numpad
    (144, k("Num Lock", "")),
    (111, k("Num /", "sfx.slash_forward")),
    (106, k("Num *", "sfx.asterisk")),
    (109, k("Num -", "")),
    (107, k("Num +", "")),
    (110, k("Num .", "sfx.default")),
    (96, k("Num 0", "&.0")), (97, k("Num 1", "&.1")), (98, k("Num 2", "&.2")),
    (99, k("Num 3", "&.3")), (100, k("Num 4", "&.4")), (101, k("Num 5", "&.5")),
    (102, k("Num 6", "&.6")), (103, k("Num 7", "&.7")), (104, k("Num 8", "&.8")),
    (105, k("Num 9", "&.9")),
    // Media
    (173, k("Vol Mute", "")), (174, k("Vol Down", "")), (175, k("Vol Up", "")),
    (176, k("Next", "")), (177, k("Prev", "")), (179, k("Play", "")),
    // Other
    (145, k("Scroll Lock", "")),
    (19, k("Pause", "")),
    (44, k("Print Screen", "")),
];
