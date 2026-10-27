// The text of an item's tag, centred on the item: one line per field, stacked about its centre.

const MAX_LINE = 24;
const clip = (s: string): string => (s.length > MAX_LINE ? `${s.slice(0, MAX_LINE - 1)}…` : s);

export function TagLabel({ x, y, fontSize, lines }: { x: number; y: number; fontSize: number; lines: readonly string[] }) {
  if (lines.length === 0) return null;
  const first = -((lines.length - 1) / 2) * 1.15;
  return (
    <text className="tag" x={x} y={y} fontSize={fontSize} textAnchor="middle" dominantBaseline="central">
      {lines.map((line, i) => (
        <tspan key={i} x={x} dy={i === 0 ? `${first}em` : "1.15em"}>
          {clip(line)}
        </tspan>
      ))}
    </text>
  );
}
