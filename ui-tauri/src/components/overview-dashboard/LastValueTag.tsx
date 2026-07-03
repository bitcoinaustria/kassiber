type LastValueTagViewBox = {
  x?: number;
  y?: number;
  width?: number;
  height?: number;
};

// Text color by fill luminance, not `var(--background)`: the fill is a fixed
// series color while the background flips with the theme, so white-on-orange
// (light mode) would land around 2:1 contrast.
function readableTextColor(fill: string) {
  const hex = fill.replace("#", "");
  if (hex.length !== 6) return "#ffffff";
  const r = parseInt(hex.slice(0, 2), 16);
  const g = parseInt(hex.slice(2, 4), 16);
  const b = parseInt(hex.slice(4, 6), 16);
  return (r * 299 + g * 587 + b * 114) / 1000 >= 140 ? "#09090b" : "#ffffff";
}

// Small muted text above/below a ReferenceDot — used for the on-chart
// high/low annotations. recharts 3 dropped the object-form `label` prop,
// so this is a render function like the tag below.
export function renderPointValueLabel({
  text,
  side,
}: {
  text: string;
  side: "above" | "below";
}) {
  return function PointValueLabel(props: unknown) {
    const viewBox = (props as { viewBox?: LastValueTagViewBox }).viewBox;
    const x = (viewBox?.x ?? 0) + (viewBox?.width ?? 0) / 2;
    const y =
      side === "above"
        ? (viewBox?.y ?? 0) - 5
        : (viewBox?.y ?? 0) + (viewBox?.height ?? 0) + 5;
    return (
      <text
        x={x}
        y={y}
        dy={side === "above" ? 0 : "0.71em"}
        textAnchor="middle"
        fontSize={10}
        fill="var(--muted-foreground)"
        pointerEvents="none"
      >
        {text}
      </text>
    );
  };
}

// Renders the TradingView-style value tag a ReferenceLine anchors on the
// axis: a small filled pill with the latest series value, sitting on the
// left (BTC) or right (fiat price) edge of the plot. Used as the
// ReferenceLine `label` render prop, so it draws in SVG coordinates.
export function renderLastValueTag({
  text,
  fill,
  side,
}: {
  text: string;
  fill: string;
  side: "left" | "right";
}) {
  return function LastValueTagLabel(props: unknown) {
    const viewBox = (props as { viewBox?: LastValueTagViewBox }).viewBox;
    const x = viewBox?.x ?? 0;
    const y = viewBox?.y ?? 0;
    const width = viewBox?.width ?? 0;
    const tagHeight = 15;
    const tagWidth = Math.max(30, Math.round(text.length * 5.8) + 10);
    const tagX = side === "right" ? x + width + 2 : x + 2;
    return (
      <g pointerEvents="none">
        <rect
          x={tagX}
          y={y - tagHeight / 2}
          width={tagWidth}
          height={tagHeight}
          rx={3}
          fill={fill}
        />
        <text
          x={tagX + tagWidth / 2}
          y={y}
          dy="0.34em"
          textAnchor="middle"
          fontSize={9.5}
          fontWeight={600}
          fill={readableTextColor(fill)}
        >
          {text}
        </text>
      </g>
    );
  };
}
