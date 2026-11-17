import { useEffect, useRef } from "react";
import { MergeView } from "@codemirror/merge";
import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { sanitizeHttpUrl } from "../ipc/tauriBridge";
import type {
  DiffDocument,
  MarkdownBlockNode,
  MarkdownDocumentAst,
  MarkdownInlineNode,
} from "../ipc/types";

type DiffAndReportPanelProps = {
  diff: DiffDocument;
  reportAst: MarkdownDocumentAst;
};

function renderInlineNodes(nodes: readonly MarkdownInlineNode[], keyPrefix: string) {
  return nodes.map((node, index) => {
    const key = `${keyPrefix}-i-${String(index)}`;
    switch (node.kind) {
      case "text":
        return <span key={key}>{node.text}</span>;
      case "strong":
        return <strong key={key}>{renderInlineNodes(node.children, key)}</strong>;
      case "emphasis":
        return <em key={key}>{renderInlineNodes(node.children, key)}</em>;
      case "strikethrough":
        return <del key={key}>{renderInlineNodes(node.children, key)}</del>;
      case "code":
        return (
          <code key={key} className="mono">
            {node.text}
          </code>
        );
      case "link": {
        const safeHref = sanitizeHttpUrl(node.href);
        if (safeHref === null) {
          return (
            <span key={key} className="badge badge-danger">
              [URL non sûre neutralisée] {renderInlineNodes(node.children, key)}
            </span>
          );
        }
        return (
          <a
            key={key}
            href={safeHref}
            title={node.title ?? undefined}
            rel="noopener noreferrer"
            target="_blank"
          >
            {renderInlineNodes(node.children, key)}
          </a>
        );
      }
      case "soft_break":
        return " ";
      case "hard_break":
        return <br key={key} />;
    }
  });
}

function renderBlockNodes(blocks: readonly MarkdownBlockNode[], keyPrefix: string) {
  return blocks.map((block, index) => {
    const key = `${keyPrefix}-b-${String(index)}`;
    switch (block.kind) {
      case "heading": {
        const content = renderInlineNodes(block.children, key);
        if (block.level === 1) return <h3 key={key}>{content}</h3>;
        if (block.level === 2) return <h4 key={key}>{content}</h4>;
        return <h5 key={key}>{content}</h5>;
      }
      case "paragraph":
        return <p key={key}>{renderInlineNodes(block.children, key)}</p>;
      case "list": {
        const listItems = block.items.map((itemBlocks, itemIdx) => (
          <li key={`${key}-li-${String(itemIdx)}`}>
            {renderBlockNodes(itemBlocks, `${key}-li-${String(itemIdx)}`)}
          </li>
        ));
        return block.ordered ? (
          <ol key={key} start={block.start}>
            {listItems}
          </ol>
        ) : (
          <ul key={key}>{listItems}</ul>
        );
      }
      case "blockquote":
        return (
          <blockquote
            key={key}
            style={{
              margin: "8px 0",
              paddingLeft: "12px",
              borderLeft: "3px solid var(--color-accent)",
            }}
          >
            {renderBlockNodes(block.children, key)}
          </blockquote>
        );
      case "code_block":
        return (
          <pre
            key={key}
            className="mono"
            style={{
              padding: "8px",
              borderRadius: "8px",
              backgroundColor: "var(--color-surface-subtle)",
              overflowX: "auto",
            }}
          >
            <code>{block.code}</code>
          </pre>
        );
      case "thematic_break":
        return <hr key={key} />;
      case "table":
        return (
          <table key={key} style={{ width: "100%", borderCollapse: "collapse" }}>
            <thead>
              <tr>
                {block.headers.map((cell, cIdx) => (
                  <th
                    key={`${key}-th-${String(cIdx)}`}
                    style={{ textAlign: "left", borderBottom: "1px solid var(--color-border)" }}
                  >
                    {renderInlineNodes(cell, `${key}-th-${String(cIdx)}`)}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {block.rows.map((row, rIdx) => (
                <tr key={`${key}-tr-${String(rIdx)}`}>
                  {row.map((cell, cIdx) => (
                    <td
                      key={`${key}-td-${String(rIdx)}-${String(cIdx)}`}
                      style={{ borderBottom: "1px solid var(--color-border)", padding: "4px 0" }}
                    >
                      {renderInlineNodes(cell, `${key}-td-${String(rIdx)}-${String(cIdx)}`)}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        );
    }
  });
}

export function DiffAndReportPanel({ diff, reportAst }: DiffAndReportPanelProps) {
  const mergeHostRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const container = mergeHostRef.current;
    if (!container) {
      return;
    }

    const view = new MergeView({
      a: {
        doc: diff.originalText,
        extensions: [
          EditorState.readOnly.of(true),
          EditorView.editable.of(false),
          EditorView.contentAttributes.of({ "aria-label": "Révision de base du diff" }),
        ],
      },
      b: {
        doc: diff.modifiedText,
        extensions: [
          EditorState.readOnly.of(true),
          EditorView.editable.of(false),
          EditorView.contentAttributes.of({ "aria-label": "Révision cible du diff" }),
        ],
      },
      parent: container,
    });

    return () => {
      view.destroy();
    };
  }, [diff]);

  return (
    <div className="split-grid">
      <section className="panel-card" aria-labelledby="diff-panel-title">
        <div className="panel-header">
          <div>
            <h2 id="diff-panel-title" className="panel-title">
              {diff.title} ({diff.totalLines.toLocaleString("fr-FR")} lignes)
            </h2>
            <p className="mono" style={{ margin: "4px 0 0 0" }}>
              {diff.path} ({diff.baseRevision} → {diff.targetRevision})
            </p>
          </div>
          <span className="badge">CodeMirror Merge (lecture seule)</span>
        </div>
        <div
          ref={mergeHostRef}
          className="codemirror-host"
          data-testid="codemirror-diff-host"
          aria-label={`Diff CodeMirror de ${String(diff.totalLines)} lignes`}
        />
      </section>

      <section className="panel-card" aria-labelledby="report-panel-title">
        <div className="panel-header">
          <div>
            <h2 id="report-panel-title" className="panel-title">
              Rapport Markdown (AST Rust borné)
            </h2>
            <p className="mono" style={{ margin: "4px 0 0 0" }}>
              Nœuds : {reportAst.total_nodes} · HTML bloqué : {reportAst.blocked_raw_html_count} ·
              URLs bloquées : {reportAst.blocked_url_count}
            </p>
          </div>
          {reportAst.truncated ? (
            <span className="badge badge-attention">AST tronqué au plafond</span>
          ) : (
            <span className="badge badge-success">Rendu React sûr (zéro HTML brut)</span>
          )}
        </div>
        <div className="markdown-ast-view" data-testid="markdown-ast-report">
          {renderBlockNodes(reportAst.blocks, "report")}
        </div>
      </section>
    </div>
  );
}
