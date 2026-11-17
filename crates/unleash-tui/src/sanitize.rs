// SPDX-License-Identifier: Apache-2.0
//! Neutralisation des séquences de contrôle ANSI/OSC/DCS/C0/C1 dans les vues textuelles TUI
//! (`REQ-CLI-007`, `AC-F015-007`).

/// Assainit une chaîne issue d'un log, événement, nom d'agent ou document non fiable en supprimant
/// toutes les séquences d'échappement ANSI/CSI/OSC/DCS/APC ainsi que les caractères de contrôle
/// C0/C1 dangereux, tout en préservant l'UTF-8 valide et les retours à la ligne (`\n`).
#[must_use]
pub fn sanitize_terminal_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\x1b' => {
                // Ignorer la séquence d'échappement complète (CSI, OSC, DCS, APC ou escape court).
                match chars.peek().copied() {
                    Some('[') => {
                        // CSI : ESC [ ... jusqu'à octet final 0x40..=0x7E
                        let _ = chars.next();
                        for c in chars.by_ref() {
                            if ('@'..='~').contains(&c) {
                                break;
                            }
                        }
                    }
                    Some(']' | 'P' | '_' | '^' | 'X') => {
                        // OSC / DCS / APC / PM / SOS : terminé par BEL (\x07) ou ST (ESC \)
                        let _ = chars.next();
                        while let Some(c) = chars.next() {
                            if c == '\x07' {
                                break;
                            }
                            if c == '\x1b' && chars.peek() == Some(&'\\') {
                                let _ = chars.next();
                                break;
                            }
                        }
                    }
                    Some(_) => {
                        let _ = chars.next();
                    }
                    None => {}
                }
            }
            '\n' => out.push('\n'),
            '\t' => out.push_str("    "),
            '\r' => {}
            c if c.is_control() => {}
            c => out.push(c),
        }
    }

    out
}

/// Assainit une ligne unique en supprimant aussi les sauts de ligne pour tableaux/arbres.
#[must_use]
pub fn sanitize_single_line(input: &str) -> String {
    sanitize_terminal_text(input).replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::{sanitize_single_line, sanitize_terminal_text};

    #[test]
    fn neutralizes_ansi_osc8_osc52_and_c0_sequences() {
        let hostile = "ok \x1b[31mROUGE\x1b[0m \x1b]8;;https://evil.test\x07lien\x1b]8;;\x07 \x1b]52;c;c2VjcmV0\x07fin\x00\x08";
        let clean = sanitize_terminal_text(hostile);
        assert!(!clean.contains('\x1b'));
        assert!(!clean.contains('\x07'));
        assert!(!clean.contains('\x00'));
        assert_eq!(clean, "ok ROUGE lien fin");
    }

    #[test]
    fn preserves_utf8_accents_and_cjk() {
        let text = "Équipe 日本語 & vérification\nligne 2";
        assert_eq!(sanitize_terminal_text(text), text);
        assert_eq!(
            sanitize_single_line(text),
            "Équipe 日本語 & vérification ligne 2"
        );
    }
}
