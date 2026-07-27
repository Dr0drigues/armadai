/* ArmadAI Icon System — Lucide geometry (self-hosted, 24×24 viewBox) */

export type Node = {
  tag: "path" | "circle" | "line" | "polyline" | "polygon" | "rect";
  attrs: Record<string, string | number>;
};

export const ICONS: Record<string, Node[]> = {
  agents: [
    {
      tag: "circle",
      attrs: { cx: 12, cy: 12, r: 3 },
    },
    {
      tag: "path",
      attrs: {
        d: "M12 2v4M12 18v4M2 12h4M18 12h4M5.636 5.636l2.828 2.828M15.536 15.536l2.828 2.828M18.364 5.636l-2.828 2.828M8.464 15.536l-2.828 2.828",
      },
    },
  ],

  prompts: [
    {
      tag: "path",
      attrs: {
        d: "M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z",
      },
    },
  ],

  skills: [
    {
      tag: "path",
      attrs: {
        d: "M12 2L15.09 8.26L22 9.27L17 14.14L18.18 21.02L12 17.77L5.82 21L6.91 14.09L2 9.36L8.91 8.45L12 2Z",
      },
    },
  ],

  starters: [
    {
      tag: "path",
      attrs: {
        d: "M11 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2v6m-9-6v14m6-14l-4 4m0 0l4 4m-4-4l4-4m0 0l-4-4",
      },
    },
  ],

  history: [
    {
      tag: "circle",
      attrs: { cx: 12, cy: 12, r: 10 },
    },
    {
      tag: "polyline",
      attrs: { points: "12 6 12 12 16 14" },
    },
  ],

  costs: [
    {
      tag: "line",
      attrs: { x1: 12, y1: 2, x2: 12, y2: 22 },
    },
    {
      tag: "path",
      attrs: {
        d: "M17 5H9.5a3.5 3.5 0 0 0 0 7h5a3.5 3.5 0 0 1 0 7H6",
      },
    },
  ],

  models: [
    {
      tag: "path",
      attrs: {
        d: "M3 9l9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z",
      },
    },
    {
      tag: "polyline",
      attrs: { points: "9 22 9 12 15 12 15 22" },
    },
  ],

  orchestration: [
    {
      tag: "circle",
      attrs: { cx: 12, cy: 12, r: 2 },
    },
    {
      tag: "circle",
      attrs: { cx: 5, cy: 6, r: 2 },
    },
    {
      tag: "circle",
      attrs: { cx: 19, cy: 6, r: 2 },
    },
    {
      tag: "circle",
      attrs: { cx: 5, cy: 18, r: 2 },
    },
    {
      tag: "circle",
      attrs: { cx: 19, cy: 18, r: 2 },
    },
    {
      tag: "path",
      attrs: {
        d: "M12 12L5 6M12 12L19 6M12 12L5 18M12 12L19 18",
      },
    },
  ],
};
