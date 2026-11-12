import { useEffect, useRef } from "react";
import cytoscape from "cytoscape";
import type { Core } from "cytoscape";
import { download, exportJSON } from "./api";
import type { Envelope, Graph } from "./api";
// DOM text nodes escape markup, but XML 1.0 also forbids some valid source-text
// characters (and lone UTF-16 surrogates introduced by label truncation).
function xmlText(value: string): string {
  return Array.from(value, (character) => {
    const code = character.codePointAt(0)!;
    return code === 9 ||
      code === 10 ||
      code === 13 ||
      (code >= 0x20 && code <= 0xd7ff) ||
      (code >= 0xe000 && code <= 0xfffd) ||
      (code >= 0x10000 && code <= 0x10ffff)
      ? character
      : "\uFFFD";
  }).join("");
}
export function GraphView({
  result,
  onSelect,
  dark,
}: {
  result: Envelope<Graph> | null;
  onSelect: (id: string) => void;
  dark: boolean;
}) {
  const container = useRef<HTMLDivElement>(null);
  const instance = useRef<Core | null>(null);
  const select = useRef(onSelect);
  select.current = onSelect;
  useEffect(() => {
    if (!container.current || !result) return;
    const nodes = result.data.nodes.slice(0, 200);
    const ids = new Set(nodes.map((n) => n.symbol_id));
    const edges = result.data.edges
      .filter(
        (e) => e.target_id && ids.has(e.source_id) && ids.has(e.target_id),
      )
      .slice(0, 1000);
    const cy = cytoscape({
      container: container.current,
      elements: [
        ...nodes.map((n) => ({
          data: { id: n.symbol_id, label: n.name.slice(0, 64), kind: n.kind },
        })),
        ...edges.map((e) => ({
          data: {
            id: e.id,
            source: e.source_id,
            target: e.target_id!,
            label: e.kind,
            confidence: e.confidence,
          },
        })),
      ],
      layout: { name: "cose", animate: false, randomize: false },
      minZoom: 0.1,
      maxZoom: 3,
      wheelSensitivity: 0.2,
      style: [
        {
          selector: "node",
          style: {
            "background-color": dark ? "#74c7ec" : "#176b9a",
            label: "data(label)",
            color: dark ? "#e8edf6" : "#162437",
            "font-size": 12,
            "text-valign": "bottom",
            "text-margin-y": 6,
            width: 24,
            height: 24,
          },
        },
        {
          selector: "edge",
          style: {
            "line-color": dark ? "#71819b" : "#8a9cb1",
            "target-arrow-color": dark ? "#71819b" : "#8a9cb1",
            "target-arrow-shape": "triangle",
            "curve-style": "bezier",
            width: 1.5,
          },
        },
        {
          selector: ":selected",
          style: {
            "background-color": "#f3a74d",
            "line-color": "#f3a74d",
            "target-arrow-color": "#f3a74d",
            "border-width": 3,
            "border-color": "#f3a74d",
          },
        },
      ],
    });
    instance.current = cy;
    cy.on("tap", "node", (event) => select.current(event.target.id()));
    const resize = new ResizeObserver(() => cy.resize());
    resize.observe(container.current);
    return () => {
      resize.disconnect();
      cy.destroy();
      instance.current = null;
    };
  }, [result, dark]);
  function svg() {
    const cy = instance.current;
    if (!cy || !result) return;
    const ns = "http://www.w3.org/2000/svg";
    const root = document.createElementNS(ns, "svg");
    root.setAttribute("xmlns", ns);
    const box = cy.elements().boundingBox();
    const pad = 60;
    root.setAttribute(
      "viewBox",
      `${box.x1 - pad} ${box.y1 - pad} ${box.w + pad * 2} ${box.h + pad * 2}`,
    );
    const desc = document.createElementNS(ns, "desc");
    desc.textContent = xmlText(
      `Kortex indexed graph ${result.repo_id}, generation ${result.generation_id}. Only loaded bounded evidence.`,
    );
    root.append(desc);
    const background = document.createElementNS(ns, "rect");
    for (const [key, value] of Object.entries({
      x: box.x1 - pad,
      y: box.y1 - pad,
      width: box.w + pad * 2,
      height: box.h + pad * 2,
      fill: dark ? "#131e30" : "#ffffff",
    }))
      background.setAttribute(key, String(value));
    root.append(background);
    const defs = document.createElementNS(ns, "defs");
    const marker = document.createElementNS(ns, "marker");
    for (const [key, value] of Object.entries({
      id: "kortex-arrow",
      viewBox: "0 0 10 10",
      refX: "10",
      refY: "5",
      markerWidth: "7",
      markerHeight: "7",
      orient: "auto",
    }))
      marker.setAttribute(key, value);
    const arrow = document.createElementNS(ns, "path");
    arrow.setAttribute("d", "M 0 0 L 10 5 L 0 10 z");
    arrow.setAttribute("fill", "#71819b");
    marker.append(arrow);
    defs.append(marker);
    root.append(defs);
    cy.edges().forEach((e) => {
      const line = document.createElementNS(ns, "line");
      const from = e.source().position(),
        to = e.target().position();
      const length = Math.hypot(to.x - from.x, to.y - from.y) || 1;
      for (const [k, v] of Object.entries({
        x1: from.x,
        y1: from.y,
        x2: to.x - ((to.x - from.x) / length) * 15,
        y2: to.y - ((to.y - from.y) / length) * 15,
        stroke: "#71819b",
        "stroke-width": 1.5,
        "marker-end": "url(#kortex-arrow)",
      }))
        line.setAttribute(k, String(v));
      const evidence = result.data.edges.find((edge) => edge.id === e.id());
      const title = document.createElementNS(ns, "title");
      title.textContent = xmlText(
        evidence
          ? `${evidence.kind} · ${evidence.resolution} · confidence ${evidence.confidence}: ${evidence.evidence}`
          : String(e.data("label")),
      );
      line.append(title);
      root.append(line);
    });
    cy.nodes().forEach((n) => {
      const p = n.position();
      const circle = document.createElementNS(ns, "circle");
      circle.setAttribute("cx", String(p.x));
      circle.setAttribute("cy", String(p.y));
      circle.setAttribute("r", "12");
      circle.setAttribute("fill", "#176b9a");
      const title = document.createElementNS(ns, "title");
      const symbol = result.data.nodes.find(
        (symbol) => symbol.symbol_id === n.id(),
      );
      title.textContent = xmlText(
        symbol
          ? `${symbol.qualified_name} (${symbol.file_path}:${symbol.span.start_line}) · ${n.id()}`
          : n.id(),
      );
      circle.append(title);
      root.append(circle);
      const label = document.createElementNS(ns, "text");
      label.setAttribute("x", String(p.x + 16));
      label.setAttribute("y", String(p.y + 4));
      label.setAttribute("fill", dark ? "#e8edf6" : "#162437");
      label.textContent = xmlText(String(n.data("label")));
      root.append(label);
    });
    download(
      new Blob([new XMLSerializer().serializeToString(root)], {
        type: "image/svg+xml",
      }),
      "kortex-graph.svg",
    );
  }
  return (
    <section aria-labelledby="graph-heading" className="graph-panel">
      <div className="toolbar">
        <h2 id="graph-heading">Bounded graph</h2>
        <button
          disabled={!result}
          onClick={() => instance.current?.fit(undefined, 40)}
        >
          Fit graph
        </button>
        <button disabled={!result} onClick={svg}>
          Export SVG
        </button>
        <button
          disabled={!result}
          onClick={() => {
            const cy = instance.current;
            if (cy)
              download(
                cy.png({
                  output: "blob",
                  full: true,
                  scale: 2,
                  maxWidth: 4096,
                  maxHeight: 4096,
                  bg: dark ? "#131e30" : "#ffffff",
                }),
                "kortex-graph.png",
              );
          }}
        >
          Export PNG
        </button>
        <button
          disabled={!result}
          onClick={() => exportJSON(result, "kortex-graph.json")}
        >
          Export JSON
        </button>
      </div>
      <p className="muted">
        Up to 200 nodes / 1,000 displayed edges. Use the keyboard node list
        below for equivalent navigation.
      </p>
      <div
        className="graph"
        ref={container}
        role="img"
        aria-label={
          result
            ? `Dependency graph with ${Math.min(result.data.nodes.length, 200)} indexed nodes`
            : "Select an indexed symbol to load its graph"
        }
      />
      {result && (
        <>
          <p>
            {result.data.nodes.length} loaded nodes · {result.data.edges.length}{" "}
            evidence edges{result.truncated ? " · Traversal truncated" : ""}
          </p>
          <details>
            <summary>Keyboard graph navigation</summary>
            <ul className="node-list">
              {result.data.nodes.slice(0, 200).map((n) => (
                <li key={n.symbol_id}>
                  <button onClick={() => onSelect(n.symbol_id)}>
                    {n.qualified_name} · {n.file_path}:{n.span.start_line}
                  </button>
                </li>
              ))}
            </ul>
          </details>
        </>
      )}
    </section>
  );
}
