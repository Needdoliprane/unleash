// SPDX-License-Identifier: Apache-2.0
//! Parseur Markdown vers AST borné côté Rust (`pulldown-cmark`, `ADR-03` amendement R7).
//!
//! Tout HTML brut, SVG embarqué ou URL à schéma non sûr (`javascript:`, `data:`, `file:`,
//! `vbscript:`) est neutralisé avant sérialisation vers la `WebView`.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::errors::GuiError;

/// Taille maximale acceptée en entrée pour un document Markdown (256 KiB).
pub const MAX_MARKDOWN_INPUT_BYTES: usize = 256 * 1024;
/// Nombre maximal de nœuds AST générés pour éviter tout épuisement mémoire `WebView`.
pub const MAX_AST_NODES: usize = 4_096;
/// Profondeur d'imbrication maximale autorisée dans l'AST.
pub const MAX_AST_DEPTH: usize = 16;

/// Nœud en ligne sûr de l'AST Markdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MarkdownInlineNode {
    /// Texte littéral échappé par React.
    Text {
        /// Contenu textuel brut.
        text: String,
    },
    /// Emphase forte (`**...**`).
    Strong {
        /// Enfants en ligne.
        children: Vec<MarkdownInlineNode>,
    },
    /// Emphase (`*...*`).
    Emphasis {
        /// Enfants en ligne.
        children: Vec<MarkdownInlineNode>,
    },
    /// Texte barré (`~~...~~`).
    Strikethrough {
        /// Enfants en ligne.
        children: Vec<MarkdownInlineNode>,
    },
    /// Code en ligne (`` `...` ``).
    Code {
        /// Texte littéral de code.
        text: String,
    },
    /// Lien validé (`http://` ou `https://` uniquement).
    Link {
        /// URL validée et normalisée.
        href: String,
        /// Titre optionnel du lien.
        title: Option<String>,
        /// Libellé du lien.
        children: Vec<MarkdownInlineNode>,
    },
    /// Saut de ligne doux.
    SoftBreak,
    /// Saut de ligne dur.
    HardBreak,
}

/// Nœud bloc sûr de l'AST Markdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MarkdownBlockNode {
    /// Titre de niveau `1..=6`.
    Heading {
        /// Niveau de titre (`1` à `6`).
        level: u8,
        /// Enfants en ligne.
        children: Vec<MarkdownInlineNode>,
    },
    /// Paragraphe.
    Paragraph {
        /// Enfants en ligne.
        children: Vec<MarkdownInlineNode>,
    },
    /// Liste ordonnée ou non ordonnée.
    List {
        /// `true` si liste numérotée.
        ordered: bool,
        /// Numéro de départ pour les listes ordonnées.
        #[ts(type = "number")]
        start: u64,
        /// Éléments de liste.
        items: Vec<Vec<MarkdownBlockNode>>,
    },
    /// Citation.
    Blockquote {
        /// Blocs contenus dans la citation.
        children: Vec<MarkdownBlockNode>,
    },
    /// Bloc de code inerte.
    CodeBlock {
        /// Langage indiqué ou `None`.
        language: Option<String>,
        /// Contenu littéral du code.
        code: String,
    },
    /// Séparateur horizontal.
    ThematicBreak,
    /// Tableau structuré.
    Table {
        /// Cellules d'en-tête.
        headers: Vec<Vec<MarkdownInlineNode>>,
        /// Lignes de données.
        rows: Vec<Vec<Vec<MarkdownInlineNode>>>,
    },
}

/// Document AST borné prêt pour le rendu React sans `dangerouslySetInnerHTML`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct MarkdownDocumentAst {
    /// Indique si le document a été borné au plafond [`MAX_AST_NODES`].
    pub truncated: bool,
    /// Nombre total de nœuds produits.
    #[ts(type = "number")]
    pub total_nodes: usize,
    /// Nombre de fragments HTML/SVG bruts neutralisés.
    #[ts(type = "number")]
    pub blocked_raw_html_count: usize,
    /// Nombre d'URLs à schéma non autorisé neutralisées.
    #[ts(type = "number")]
    pub blocked_url_count: usize,
    /// Liste ordonnée des blocs de premier niveau.
    pub blocks: Vec<MarkdownBlockNode>,
}

#[derive(Debug)]
enum Frame {
    Heading {
        level: u8,
        inlines: Vec<MarkdownInlineNode>,
    },
    Paragraph {
        inlines: Vec<MarkdownInlineNode>,
    },
    Blockquote {
        blocks: Vec<MarkdownBlockNode>,
    },
    List {
        ordered: bool,
        start: u64,
        items: Vec<Vec<MarkdownBlockNode>>,
    },
    ListItem {
        blocks: Vec<MarkdownBlockNode>,
        pending_inlines: Vec<MarkdownInlineNode>,
    },
    CodeBlock {
        language: Option<String>,
        code: String,
    },
    Table {
        headers: Vec<Vec<MarkdownInlineNode>>,
        rows: Vec<Vec<Vec<MarkdownInlineNode>>>,
    },
    TableHead {
        cells: Vec<Vec<MarkdownInlineNode>>,
    },
    TableRow {
        cells: Vec<Vec<MarkdownInlineNode>>,
    },
    TableCell {
        inlines: Vec<MarkdownInlineNode>,
    },
    Strong {
        inlines: Vec<MarkdownInlineNode>,
    },
    Emphasis {
        inlines: Vec<MarkdownInlineNode>,
    },
    Strikethrough {
        inlines: Vec<MarkdownInlineNode>,
    },
    Link {
        safe_href: Option<String>,
        title: Option<String>,
        inlines: Vec<MarkdownInlineNode>,
    },
    Image {
        inlines: Vec<MarkdownInlineNode>,
    },
}

/// Valide qu'une URL utilise exclusivement `http://` ou `https://` et aucun caractère de contrôle.
#[must_use]
pub fn sanitize_external_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.chars().any(char::is_control) {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") {
        Some(trimmed.to_owned())
    } else {
        None
    }
}

const fn heading_level_num(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Analyse un document Markdown brut en AST sûr et borné pour le renderer React.
///
/// # Errors
/// Retourne [`GuiError::InvalidInput`] si l'entrée dépasse [`MAX_MARKDOWN_INPUT_BYTES`].
pub fn parse_markdown_ast(input: &str) -> Result<MarkdownDocumentAst, GuiError> {
    if input.len() > MAX_MARKDOWN_INPUT_BYTES {
        return Err(GuiError::InvalidInput {
            detail: format!(
                "Document Markdown trop volumineux ({} octets > plafond {})",
                input.len(),
                MAX_MARKDOWN_INPUT_BYTES
            ),
        });
    }

    let options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES;
    let parser = Parser::new_ext(input, options);

    let mut root_blocks: Vec<MarkdownBlockNode> = Vec::new();
    let mut stack: Vec<Frame> = Vec::new();
    let mut total_nodes = 0usize;
    let mut truncated = false;
    let mut blocked_raw_html_count = 0usize;
    let mut blocked_url_count = 0usize;

    for event in parser {
        if total_nodes >= MAX_AST_NODES {
            truncated = true;
            break;
        }

        match event {
            Event::Start(tag) => {
                if stack.len() >= MAX_AST_DEPTH {
                    truncated = true;
                    continue;
                }
                handle_start_tag(tag, &mut stack, &mut blocked_url_count);
            }
            Event::End(tag_end) => {
                if !matches_top_frame(stack.last(), tag_end) {
                    continue;
                }
                let Some(frame) = stack.pop() else {
                    continue;
                };
                total_nodes += 1;
                commit_frame(frame, &mut stack, &mut root_blocks);
            }
            other => handle_leaf_event(
                other,
                &mut stack,
                &mut root_blocks,
                &mut total_nodes,
                &mut blocked_raw_html_count,
            ),
        }
    }

    while let Some(frame) = stack.pop() {
        commit_frame(frame, &mut stack, &mut root_blocks);
    }

    Ok(MarkdownDocumentAst {
        truncated,
        total_nodes,
        blocked_raw_html_count,
        blocked_url_count,
        blocks: root_blocks,
    })
}

fn handle_start_tag(tag: Tag<'_>, stack: &mut Vec<Frame>, blocked_url_count: &mut usize) {
    match tag {
        Tag::Heading { level, .. } => stack.push(Frame::Heading {
            level: heading_level_num(level),
            inlines: Vec::new(),
        }),
        Tag::Paragraph => stack.push(Frame::Paragraph {
            inlines: Vec::new(),
        }),
        Tag::BlockQuote(_) => stack.push(Frame::Blockquote { blocks: Vec::new() }),
        Tag::List(start) => stack.push(Frame::List {
            ordered: start.is_some(),
            start: start.unwrap_or(1),
            items: Vec::new(),
        }),
        Tag::Item => stack.push(Frame::ListItem {
            blocks: Vec::new(),
            pending_inlines: Vec::new(),
        }),
        Tag::CodeBlock(kind) => {
            let language = match kind {
                CodeBlockKind::Fenced(lang) if !lang.trim().is_empty() => {
                    Some(lang.trim().to_owned())
                }
                CodeBlockKind::Fenced(_) | CodeBlockKind::Indented => None,
            };
            stack.push(Frame::CodeBlock {
                language,
                code: String::new(),
            });
        }
        Tag::Table(_) => stack.push(Frame::Table {
            headers: Vec::new(),
            rows: Vec::new(),
        }),
        Tag::TableHead => stack.push(Frame::TableHead { cells: Vec::new() }),
        Tag::TableRow => stack.push(Frame::TableRow { cells: Vec::new() }),
        Tag::TableCell => stack.push(Frame::TableCell {
            inlines: Vec::new(),
        }),
        Tag::Strong => stack.push(Frame::Strong {
            inlines: Vec::new(),
        }),
        Tag::Emphasis => stack.push(Frame::Emphasis {
            inlines: Vec::new(),
        }),
        Tag::Strikethrough => stack.push(Frame::Strikethrough {
            inlines: Vec::new(),
        }),
        Tag::Link {
            dest_url, title, ..
        } => {
            let safe_href = sanitize_external_url(&dest_url);
            if safe_href.is_none() {
                *blocked_url_count += 1;
            }
            let title = if title.trim().is_empty() {
                None
            } else {
                Some(title.into_string())
            };
            stack.push(Frame::Link {
                safe_href,
                title,
                inlines: Vec::new(),
            });
        }
        Tag::Image { .. } => stack.push(Frame::Image {
            inlines: Vec::new(),
        }),
        Tag::HtmlBlock
        | Tag::FootnoteDefinition(_)
        | Tag::MetadataBlock(_)
        | Tag::Superscript
        | Tag::Subscript
        | Tag::DefinitionList
        | Tag::DefinitionListTitle
        | Tag::DefinitionListDefinition => {}
    }
}

fn handle_leaf_event(
    event: Event<'_>,
    stack: &mut [Frame],
    root_blocks: &mut Vec<MarkdownBlockNode>,
    total_nodes: &mut usize,
    blocked_raw_html_count: &mut usize,
) {
    match event {
        Event::Text(text) => {
            *total_nodes += 1;
            if let Some(Frame::CodeBlock { code, .. }) = stack.last_mut() {
                code.push_str(&text);
            } else {
                push_inline(
                    stack,
                    root_blocks,
                    MarkdownInlineNode::Text {
                        text: text.into_string(),
                    },
                );
            }
        }
        Event::Code(code) => {
            *total_nodes += 1;
            push_inline(
                stack,
                root_blocks,
                MarkdownInlineNode::Code {
                    text: code.into_string(),
                },
            );
        }
        Event::Html(_) | Event::InlineHtml(_) => {
            *blocked_raw_html_count += 1;
            *total_nodes += 1;
            push_inline(
                stack,
                root_blocks,
                MarkdownInlineNode::Text {
                    text: String::from("[contenu HTML/SVG neutralisé]"),
                },
            );
        }
        Event::SoftBreak => {
            *total_nodes += 1;
            push_inline(stack, root_blocks, MarkdownInlineNode::SoftBreak);
        }
        Event::HardBreak => {
            *total_nodes += 1;
            push_inline(stack, root_blocks, MarkdownInlineNode::HardBreak);
        }
        Event::Rule => {
            *total_nodes += 1;
            push_block(stack, root_blocks, MarkdownBlockNode::ThematicBreak);
        }
        Event::TaskListMarker(checked) => {
            *total_nodes += 1;
            push_inline(
                stack,
                root_blocks,
                MarkdownInlineNode::Text {
                    text: if checked {
                        String::from("[x] ")
                    } else {
                        String::from("[ ] ")
                    },
                },
            );
        }
        Event::FootnoteReference(label) | Event::InlineMath(label) | Event::DisplayMath(label) => {
            *total_nodes += 1;
            push_inline(
                stack,
                root_blocks,
                MarkdownInlineNode::Text {
                    text: label.into_string(),
                },
            );
        }
        Event::Start(_) | Event::End(_) => {}
    }
}

const fn matches_top_frame(top: Option<&Frame>, end: TagEnd) -> bool {
    matches!(
        (top, end),
        (Some(Frame::Heading { .. }), TagEnd::Heading(_))
            | (Some(Frame::Paragraph { .. }), TagEnd::Paragraph)
            | (Some(Frame::Blockquote { .. }), TagEnd::BlockQuote(_))
            | (Some(Frame::List { .. }), TagEnd::List(_))
            | (Some(Frame::ListItem { .. }), TagEnd::Item)
            | (Some(Frame::CodeBlock { .. }), TagEnd::CodeBlock)
            | (Some(Frame::Table { .. }), TagEnd::Table)
            | (Some(Frame::TableHead { .. }), TagEnd::TableHead)
            | (Some(Frame::TableRow { .. }), TagEnd::TableRow)
            | (Some(Frame::TableCell { .. }), TagEnd::TableCell)
            | (Some(Frame::Strong { .. }), TagEnd::Strong)
            | (Some(Frame::Emphasis { .. }), TagEnd::Emphasis)
            | (Some(Frame::Strikethrough { .. }), TagEnd::Strikethrough)
            | (Some(Frame::Link { .. }), TagEnd::Link)
            | (Some(Frame::Image { .. }), TagEnd::Image)
    )
}

fn push_inline(
    stack: &mut [Frame],
    root_blocks: &mut Vec<MarkdownBlockNode>,
    node: MarkdownInlineNode,
) {
    match stack.last_mut() {
        Some(
            Frame::Heading { inlines, .. }
            | Frame::Paragraph { inlines }
            | Frame::TableCell { inlines }
            | Frame::Strong { inlines }
            | Frame::Emphasis { inlines }
            | Frame::Strikethrough { inlines }
            | Frame::Link { inlines, .. }
            | Frame::Image { inlines },
        ) => inlines.push(node),
        Some(Frame::ListItem {
            pending_inlines, ..
        }) => pending_inlines.push(node),
        _ => root_blocks.push(MarkdownBlockNode::Paragraph {
            children: vec![node],
        }),
    }
}

fn push_block(
    stack: &mut [Frame],
    root_blocks: &mut Vec<MarkdownBlockNode>,
    block: MarkdownBlockNode,
) {
    match stack.last_mut() {
        Some(Frame::Blockquote { blocks } | Frame::ListItem { blocks, .. }) => blocks.push(block),
        _ => root_blocks.push(block),
    }
}

fn commit_frame(frame: Frame, stack: &mut [Frame], root_blocks: &mut Vec<MarkdownBlockNode>) {
    match frame {
        Frame::Heading { .. }
        | Frame::Paragraph { .. }
        | Frame::Blockquote { .. }
        | Frame::List { .. }
        | Frame::ListItem { .. }
        | Frame::CodeBlock { .. }
        | Frame::Table { .. }
        | Frame::TableHead { .. }
        | Frame::TableRow { .. }
        | Frame::TableCell { .. } => commit_container_frame(frame, stack, root_blocks),
        Frame::Strong { .. }
        | Frame::Emphasis { .. }
        | Frame::Strikethrough { .. }
        | Frame::Link { .. }
        | Frame::Image { .. } => commit_inline_frame(frame, stack, root_blocks),
    }
}

fn commit_container_frame(
    frame: Frame,
    stack: &mut [Frame],
    root_blocks: &mut Vec<MarkdownBlockNode>,
) {
    match frame {
        Frame::Heading { level, inlines } => push_block(
            stack,
            root_blocks,
            MarkdownBlockNode::Heading {
                level,
                children: inlines,
            },
        ),
        Frame::Paragraph { inlines } => push_block(
            stack,
            root_blocks,
            MarkdownBlockNode::Paragraph { children: inlines },
        ),
        Frame::Blockquote { blocks } => push_block(
            stack,
            root_blocks,
            MarkdownBlockNode::Blockquote { children: blocks },
        ),
        Frame::List {
            ordered,
            start,
            items,
        } => push_block(
            stack,
            root_blocks,
            MarkdownBlockNode::List {
                ordered,
                start,
                items,
            },
        ),
        Frame::ListItem {
            mut blocks,
            pending_inlines,
        } => {
            if !pending_inlines.is_empty() {
                blocks.insert(
                    0,
                    MarkdownBlockNode::Paragraph {
                        children: pending_inlines,
                    },
                );
            }
            if let Some(Frame::List { items, .. }) = stack.last_mut() {
                items.push(blocks);
            } else {
                root_blocks.extend(blocks);
            }
        }
        Frame::CodeBlock { language, code } => push_block(
            stack,
            root_blocks,
            MarkdownBlockNode::CodeBlock { language, code },
        ),
        Frame::Table { headers, rows } => push_block(
            stack,
            root_blocks,
            MarkdownBlockNode::Table { headers, rows },
        ),
        Frame::TableHead { cells } => {
            if let Some(Frame::Table { headers, .. }) = stack.last_mut() {
                *headers = cells;
            }
        }
        Frame::TableRow { cells } => {
            if let Some(Frame::Table { rows, .. }) = stack.last_mut() {
                rows.push(cells);
            }
        }
        Frame::TableCell { inlines } => {
            if let Some(Frame::TableHead { cells } | Frame::TableRow { cells }) = stack.last_mut() {
                cells.push(inlines);
            }
        }
        _ => {}
    }
}

fn commit_inline_frame(
    frame: Frame,
    stack: &mut [Frame],
    root_blocks: &mut Vec<MarkdownBlockNode>,
) {
    match frame {
        Frame::Strong { inlines } => push_inline(
            stack,
            root_blocks,
            MarkdownInlineNode::Strong { children: inlines },
        ),
        Frame::Emphasis { inlines } => push_inline(
            stack,
            root_blocks,
            MarkdownInlineNode::Emphasis { children: inlines },
        ),
        Frame::Strikethrough { inlines } => push_inline(
            stack,
            root_blocks,
            MarkdownInlineNode::Strikethrough { children: inlines },
        ),
        Frame::Link {
            safe_href,
            title,
            mut inlines,
        } => {
            if let Some(href) = safe_href {
                push_inline(
                    stack,
                    root_blocks,
                    MarkdownInlineNode::Link {
                        href,
                        title,
                        children: inlines,
                    },
                );
            } else {
                inlines.push(MarkdownInlineNode::Text {
                    text: String::from(" [URL bloquée]"),
                });
                for child in inlines {
                    push_inline(stack, root_blocks, child);
                }
            }
        }
        Frame::Image { inlines } => {
            push_inline(
                stack,
                root_blocks,
                MarkdownInlineNode::Text {
                    text: String::from("[image bloquée: "),
                },
            );
            for child in inlines {
                push_inline(stack, root_blocks, child);
            }
            push_inline(
                stack,
                root_blocks,
                MarkdownInlineNode::Text {
                    text: String::from("]"),
                },
            );
        }
        _ => {}
    }
}
