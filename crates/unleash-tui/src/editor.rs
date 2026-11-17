// SPDX-License-Identifier: Apache-2.0
//! Éditeur multiligne interne `unleash-tui::editor` fondé sur `ropey` et `unicode-segmentation`
//! (`ADR-15` règle 5 : `tui-textarea` 0.7.0 reste incompatible avec Ratatui 0.30 / Crossterm 0.29).
//!
//! Supporte saisie Unicode/CJK/accents, collage contrôlé sans soumission, annuler/rétablir,
//! recherche incrémentale et rendu fenêtré Ratatui.

use std::cmp::min;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Widget};
use ropey::Rope;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::sanitize::sanitize_terminal_text;

const MAX_UNDO_HISTORY: usize = 256;

#[derive(Debug, Clone)]
struct EditorSnapshot {
    text: String,
    cursor_line: usize,
    cursor_grapheme_col: usize,
}

/// Éditeur de texte multiligne autonome avec historique annuler/rétablir et recherche.
#[derive(Debug, Clone)]
pub struct TuiEditor {
    rope: Rope,
    cursor_line: usize,
    cursor_grapheme_col: usize,
    scroll_row: usize,
    undo_stack: Vec<EditorSnapshot>,
    redo_stack: Vec<EditorSnapshot>,
    last_search: Option<(String, usize)>,
}

impl Default for TuiEditor {
    fn default() -> Self {
        Self::new("")
    }
}

impl TuiEditor {
    /// Initialise l'éditeur avec le contenu textuel fourni (assaini des séquences de contrôle).
    #[must_use]
    pub fn new(initial: &str) -> Self {
        let clean = sanitize_terminal_text(initial);
        Self {
            rope: Rope::from_str(&clean),
            cursor_line: 0,
            cursor_grapheme_col: 0,
            scroll_row: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            last_search: None,
        }
    }

    /// Retourne le texte intégral courant du tampon.
    #[must_use]
    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    /// Retourne la position logique `(ligne, colonne_graphème)` du curseur.
    #[must_use]
    pub const fn cursor(&self) -> (usize, usize) {
        (self.cursor_line, self.cursor_grapheme_col)
    }

    /// Nombre total de lignes dans le tampon.
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.rope.len_lines().max(1)
    }

    fn line_string_without_newline(&self, line_idx: usize) -> String {
        if line_idx >= self.rope.len_lines() {
            return String::new();
        }
        let slice = self.rope.line(line_idx);
        let mut s = slice.to_string();
        while s.ends_with('\n') || s.ends_with('\r') {
            s.pop();
        }
        s
    }

    fn grapheme_count_on_line(&self, line_idx: usize) -> usize {
        self.line_string_without_newline(line_idx)
            .graphemes(true)
            .count()
    }

    fn char_index_at_cursor(&self) -> usize {
        let line_idx = min(self.cursor_line, self.rope.len_lines().saturating_sub(1));
        let line_char_start = self.rope.line_to_char(line_idx);
        let line_text = self.line_string_without_newline(line_idx);
        let byte_offset: usize = line_text
            .graphemes(true)
            .take(self.cursor_grapheme_col)
            .map(str::len)
            .sum();
        let char_offset = line_text
            .get(..byte_offset)
            .map_or(0, |prefix| prefix.chars().count());
        line_char_start.saturating_add(char_offset)
    }

    fn push_undo(&mut self) {
        if self.undo_stack.len() >= MAX_UNDO_HISTORY {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(EditorSnapshot {
            text: self.rope.to_string(),
            cursor_line: self.cursor_line,
            cursor_grapheme_col: self.cursor_grapheme_col,
        });
        self.redo_stack.clear();
    }

    /// Insère un caractère Unicode à la position du curseur.
    pub fn insert_char(&mut self, ch: char) {
        if ch.is_control() && ch != '\n' {
            return;
        }
        let mut buf = [0u8; 4];
        let s = ch.encode_utf8(&mut buf);
        self.insert_str(s);
    }

    /// Insère un texte (ex. collage bracketed paste) sans jamais déclencher d'envoi métier.
    pub fn insert_str(&mut self, raw: &str) {
        let clean = sanitize_terminal_text(raw);
        if clean.is_empty() {
            return;
        }
        self.push_undo();
        let idx = self.char_index_at_cursor();
        self.rope.insert(idx, &clean);

        for ch in clean.chars() {
            if ch == '\n' {
                self.cursor_line = self.cursor_line.saturating_add(1);
                self.cursor_grapheme_col = 0;
            }
        }
        if let Some(last_segment) = clean.rsplit('\n').next()
            && !last_segment.is_empty()
        {
            if clean.contains('\n') {
                self.cursor_grapheme_col = last_segment.graphemes(true).count();
            } else {
                self.cursor_grapheme_col = self
                    .cursor_grapheme_col
                    .saturating_add(last_segment.graphemes(true).count());
            }
        }
        self.clamp_cursor();
    }

    /// Supprime le graphème immédiatement avant le curseur (Backspace).
    pub fn backspace(&mut self) {
        let idx = self.char_index_at_cursor();
        if idx == 0 {
            return;
        }
        self.push_undo();
        if self.cursor_grapheme_col > 0 {
            let line_text = self.line_string_without_newline(self.cursor_line);
            let target_col = self.cursor_grapheme_col.saturating_sub(1);
            let start_byte: usize = line_text
                .graphemes(true)
                .take(target_col)
                .map(str::len)
                .sum();
            let end_byte: usize = line_text
                .graphemes(true)
                .take(self.cursor_grapheme_col)
                .map(str::len)
                .sum();
            let line_start_char = self.rope.line_to_char(self.cursor_line);
            let start_char =
                line_start_char + line_text.get(..start_byte).map_or(0, |s| s.chars().count());
            let end_char =
                line_start_char + line_text.get(..end_byte).map_or(0, |s| s.chars().count());
            if start_char < end_char {
                self.rope.remove(start_char..end_char);
                self.cursor_grapheme_col = target_col;
            }
        } else if self.cursor_line > 0 {
            let prev_line = self.cursor_line.saturating_sub(1);
            let prev_cols = self.grapheme_count_on_line(prev_line);
            self.rope.remove((idx - 1)..idx);
            self.cursor_line = prev_line;
            self.cursor_grapheme_col = prev_cols;
        }
        self.clamp_cursor();
    }

    /// Annule la dernière modification (`Undo`). Retourne `true` si une restauration a eu lieu.
    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo_stack.pop() else {
            return false;
        };
        self.redo_stack.push(EditorSnapshot {
            text: self.rope.to_string(),
            cursor_line: self.cursor_line,
            cursor_grapheme_col: self.cursor_grapheme_col,
        });
        self.rope = Rope::from_str(&prev.text);
        self.cursor_line = prev.cursor_line;
        self.cursor_grapheme_col = prev.cursor_grapheme_col;
        self.clamp_cursor();
        true
    }

    /// Rétablit la modification annulée (`Redo`). Retourne `true` si appliquée.
    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo_stack.pop() else {
            return false;
        };
        self.undo_stack.push(EditorSnapshot {
            text: self.rope.to_string(),
            cursor_line: self.cursor_line,
            cursor_grapheme_col: self.cursor_grapheme_col,
        });
        self.rope = Rope::from_str(&next.text);
        self.cursor_line = next.cursor_line;
        self.cursor_grapheme_col = next.cursor_grapheme_col;
        self.clamp_cursor();
        true
    }

    /// Déplace le curseur vers le haut.
    pub fn move_up(&mut self) {
        self.cursor_line = self.cursor_line.saturating_sub(1);
        self.clamp_cursor();
    }

    /// Déplace le curseur vers le bas.
    pub fn move_down(&mut self) {
        let max_line = self.rope.len_lines().saturating_sub(1);
        if self.cursor_line < max_line {
            self.cursor_line += 1;
        }
        self.clamp_cursor();
    }

    /// Déplace le curseur d'un graphème vers la gauche.
    pub fn move_left(&mut self) {
        if self.cursor_grapheme_col > 0 {
            self.cursor_grapheme_col -= 1;
        } else if self.cursor_line > 0 {
            self.cursor_line -= 1;
            self.cursor_grapheme_col = self.grapheme_count_on_line(self.cursor_line);
        }
    }

    /// Déplace le curseur d'un graphème vers la droite.
    pub fn move_right(&mut self) {
        let cols = self.grapheme_count_on_line(self.cursor_line);
        if self.cursor_grapheme_col < cols {
            self.cursor_grapheme_col += 1;
        } else if self.cursor_line + 1 < self.rope.len_lines() {
            self.cursor_line += 1;
            self.cursor_grapheme_col = 0;
        }
    }

    /// Recherche `query` dans le tampon à partir de la ligne courante et déplace le curseur dessus.
    /// Retourne `Some((line_idx, grapheme_col))` si trouvée.
    pub fn search_next(&mut self, query: &str) -> Option<(usize, usize)> {
        let needle = query.trim();
        if needle.is_empty() {
            return None;
        }
        let total = self.line_count();
        let start_line = match &self.last_search {
            Some((prev, prev_line)) if prev == needle => prev_line.saturating_add(1) % total,
            _ => self.cursor_line,
        };
        for offset in 0..total {
            let line_idx = (start_line + offset) % total;
            let text = self.line_string_without_newline(line_idx);
            if let Some(byte_pos) = text.find(needle) {
                let col = text
                    .get(..byte_pos)
                    .map_or(0, |prefix| prefix.graphemes(true).count());
                self.cursor_line = line_idx;
                self.cursor_grapheme_col = col;
                self.last_search = Some((needle.to_owned(), line_idx));
                self.clamp_cursor();
                return Some((line_idx, col));
            }
        }
        None
    }

    fn clamp_cursor(&mut self) {
        let max_line = self.rope.len_lines().saturating_sub(1);
        self.cursor_line = min(self.cursor_line, max_line);
        let max_col = self.grapheme_count_on_line(self.cursor_line);
        self.cursor_grapheme_col = min(self.cursor_grapheme_col, max_col);
    }

    /// Rend l'éditeur dans `area` avec un curseur visuel explicite et respect du mode `no_color`.
    pub fn render_into(&mut self, area: Rect, buf: &mut Buffer, block: Block<'_>, no_color: bool) {
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.height == 0 || inner.width == 0 {
            return;
        }

        let visible_rows = usize::from(inner.height);
        if self.cursor_line < self.scroll_row {
            self.scroll_row = self.cursor_line;
        } else if self.cursor_line >= self.scroll_row + visible_rows {
            self.scroll_row = self.cursor_line + 1 - visible_rows;
        }

        let mut lines = Vec::with_capacity(visible_rows);
        for row_idx in self.scroll_row..min(self.line_count(), self.scroll_row + visible_rows) {
            let line_num = format!("{:>3} │ ", row_idx + 1);
            let content = self.line_string_without_newline(row_idx);
            let num_style = if no_color {
                Style::default()
            } else {
                Style::default().fg(Color::DarkGray)
            };

            if row_idx == self.cursor_line {
                let mut before = String::new();
                let mut cursor_cell = String::from(" ");
                let mut after = String::new();
                for (g_idx, g) in content.graphemes(true).enumerate() {
                    match g_idx.cmp(&self.cursor_grapheme_col) {
                        std::cmp::Ordering::Less => before.push_str(g),
                        std::cmp::Ordering::Equal => g.clone_into(&mut cursor_cell),
                        std::cmp::Ordering::Greater => after.push_str(g),
                    }
                }
                let cursor_style = if no_color {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                        .bg(Color::Cyan)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD)
                };
                let _display_w = before.width();
                lines.push(Line::from(vec![
                    Span::styled(line_num, num_style),
                    Span::raw(before),
                    Span::styled(cursor_cell, cursor_style),
                    Span::raw(after),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::styled(line_num, num_style),
                    Span::raw(content),
                ]));
            }
        }

        Paragraph::new(lines).render(inner, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::TuiEditor;

    #[test]
    fn editor_supports_unicode_cjk_undo_redo_and_search() {
        let mut ed = TuiEditor::new("ligne un\ndeux");
        ed.move_down();
        ed.insert_str(" — 日本語 éàç");
        assert!(ed.text().contains("日本語 éàç"));
        assert!(ed.undo());
        assert!(!ed.text().contains("日本語"));
        assert!(ed.redo());
        assert!(ed.text().contains("日本語 éàç"));
        let found = ed.search_next("日本語");
        assert_eq!(found, Some((1, 3)));
    }
}
