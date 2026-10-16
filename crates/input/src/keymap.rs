//! XKB keymaps for the virtual keyboard, built the way wtype builds them: every character to
//! type gets its own keycode carrying that character's keysym, so no modifiers are involved.

/// Evdev codes the paste chord uses, on their real keys so apps that match shortcuts by
/// physical key still see Ctrl and V.
pub const KEY_LEFTCTRL: u32 = 29;
pub const KEY_LEFTSHIFT: u32 = 42;
pub const KEY_V: u32 = 47;

/// The modifier masks a keymap built from `complete` assigns.
pub const MOD_SHIFT: u32 = 1;
pub const MOD_CTRL: u32 = 4;

/// Evdev keycodes start where xkb's start minus this offset.
const XKB_OFFSET: u32 = 8;

/// A keymap that types `chars` and the evdev code to press for each. The first character is
/// evdev 1 (xkb 9), the next evdev 2, and so on.
pub struct Typing {
    pub text: String,
    pub codes: Vec<u32>,
}

/// Builds the keymap for typing `text`. Characters repeat on the same key, so the keymap holds
/// each distinct one once.
pub fn typing(text: &str) -> Typing {
    let mut table: Vec<char> = Vec::new();
    let mut codes = Vec::with_capacity(text.len());
    for c in text.chars() {
        let index = match table.iter().position(|known| *known == c) {
            Some(index) => index,
            None => {
                table.push(c);
                table.len() - 1
            }
        };
        codes.push(index as u32 + 1);
    }
    let mut keycodes = String::new();
    let mut symbols = String::new();
    for (index, c) in table.iter().enumerate() {
        let evdev = index as u32 + 1;
        keycodes.push_str(&format!("<K{evdev}> = {};\n", evdev + XKB_OFFSET));
        symbols.push_str(&format!("key <K{evdev}> {{[ {} ]}};\n", keysym(*c)));
    }
    Typing {
        text: wrap(table.len() as u32 + XKB_OFFSET + 1, &keycodes, &symbols),
        codes,
    }
}

/// A keymap with Ctrl, Shift and V on their usual evdev codes, for sending the paste chord.
pub fn paste() -> String {
    let keycodes = format!(
        "<LCTL> = {};\n<LFSH> = {};\n<AB04> = {};\n",
        KEY_LEFTCTRL + XKB_OFFSET,
        KEY_LEFTSHIFT + XKB_OFFSET,
        KEY_V + XKB_OFFSET
    );
    let symbols = "key <LCTL> {[ Control_L ]};\nkey <LFSH> {[ Shift_L ]};\nkey <AB04> {[ v, V ]};\n\
                   modifier_map Control { <LCTL> };\nmodifier_map Shift { <LFSH> };\n";
    wrap(255, &keycodes, symbols)
}

fn wrap(maximum: u32, keycodes: &str, symbols: &str) -> String {
    format!(
        "xkb_keymap {{\n\
         xkb_keycodes \"(unnamed)\" {{\nminimum = 8;\nmaximum = {maximum};\n{keycodes}}};\n\
         xkb_types \"(unnamed)\" {{ include \"complete\" }};\n\
         xkb_compatibility \"(unnamed)\" {{ include \"complete\" }};\n\
         xkb_symbols \"(unnamed)\" {{\n{symbols}}};\n\
         }};\n"
    )
}

/// The keysym name for a character. Control characters with a key of their own get it, and
/// everything else uses the Unicode keysym form that xkbcommon resolves for any code point.
fn keysym(c: char) -> String {
    match c {
        '\n' => "Return".into(),
        '\t' => "Tab".into(),
        c => format!("U{:04X}", c as u32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_characters_share_a_key() {
        let typing = typing("мама a");
        assert_eq!(typing.codes, vec![1, 2, 1, 2, 3, 4]);
        assert!(typing.text.contains("key <K1> {[ U043C ]};"));
        assert!(typing.text.contains("<K4> = 12;"));
        assert!(typing.text.contains("maximum = 13;"));
    }
}
