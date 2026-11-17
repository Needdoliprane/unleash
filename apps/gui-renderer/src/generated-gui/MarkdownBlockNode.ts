// Ce fichier a été généré automatiquement par unleash-gui (ts-rs).
// Ne pas modifier manuellement.

import type { MarkdownInlineNode } from "./MarkdownInlineNode";

export type MarkdownBlockNode = { "kind": "heading", 
/**
 * Niveau de titre (`1` à `6`).
 */
level: number, 
/**
 * Enfants en ligne.
 */
children: Array<MarkdownInlineNode>, } | { "kind": "paragraph", 
/**
 * Enfants en ligne.
 */
children: Array<MarkdownInlineNode>, } | { "kind": "list", 
/**
 * `true` si liste numérotée.
 */
ordered: boolean, 
/**
 * Numéro de départ pour les listes ordonnées.
 */
start: number, 
/**
 * Éléments de liste.
 */
items: Array<Array<MarkdownBlockNode>>, } | { "kind": "blockquote", 
/**
 * Blocs contenus dans la citation.
 */
children: Array<MarkdownBlockNode>, } | { "kind": "code_block", 
/**
 * Langage indiqué ou `None`.
 */
language: string | null, 
/**
 * Contenu littéral du code.
 */
code: string, } | { "kind": "thematic_break" } | { "kind": "table", 
/**
 * Cellules d'en-tête.
 */
headers: Array<Array<MarkdownInlineNode>>, 
/**
 * Lignes de données.
 */
rows: Array<Array<Array<MarkdownInlineNode>>>, };
