// Ce fichier a été généré automatiquement par unleash-gui (ts-rs).
// Ne pas modifier manuellement.

import type { MarkdownBlockNode } from "./MarkdownBlockNode";

export type MarkdownDocumentAst = { 
/**
 * Indique si le document a été borné au plafond [`MAX_AST_NODES`].
 */
truncated: boolean, 
/**
 * Nombre total de nœuds produits.
 */
total_nodes: number, 
/**
 * Nombre de fragments HTML/SVG bruts neutralisés.
 */
blocked_raw_html_count: number, 
/**
 * Nombre d'URLs à schéma non autorisé neutralisées.
 */
blocked_url_count: number, 
/**
 * Liste ordonnée des blocs de premier niveau.
 */
blocks: Array<MarkdownBlockNode>, };
