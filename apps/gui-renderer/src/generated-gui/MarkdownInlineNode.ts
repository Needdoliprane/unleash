// Ce fichier a été généré automatiquement par unleash-gui (ts-rs).
// Ne pas modifier manuellement.

export type MarkdownInlineNode = { "kind": "text", 
/**
 * Contenu textuel brut.
 */
text: string, } | { "kind": "strong", 
/**
 * Enfants en ligne.
 */
children: Array<MarkdownInlineNode>, } | { "kind": "emphasis", 
/**
 * Enfants en ligne.
 */
children: Array<MarkdownInlineNode>, } | { "kind": "strikethrough", 
/**
 * Enfants en ligne.
 */
children: Array<MarkdownInlineNode>, } | { "kind": "code", 
/**
 * Texte littéral de code.
 */
text: string, } | { "kind": "link", 
/**
 * URL validée et normalisée.
 */
href: string, 
/**
 * Titre optionnel du lien.
 */
title: string | null, 
/**
 * Libellé du lien.
 */
children: Array<MarkdownInlineNode>, } | { "kind": "soft_break" } | { "kind": "hard_break" };
