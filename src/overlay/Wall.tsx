import { useEffect, useRef } from "react";
import { thumbUrl } from "../lib/api";
import type { Item } from "../lib/api";

interface Props {
  items: Item[];
  active: number;
  onSelect: (index: number) => void;
  onApply: (index: number) => void;
  /** reports how many columns the grid currently has (for Up/Down keys) */
  onColumns: (cols: number) => void;
}

/** Scrolling grid. Thumbnails load lazily, so only cells near the viewport hit the Rust thumbnailer. */
export function Wall({ items, active, onSelect, onApply, onColumns }: Props) {
  const gridRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const grid = gridRef.current;
    if (!grid) return;
    const measure = () => {
      const cols = getComputedStyle(grid).gridTemplateColumns.split(" ").length;
      onColumns(Math.max(1, cols));
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(grid);
    return () => ro.disconnect();
  }, [onColumns]);

  useEffect(() => {
    const cell = gridRef.current?.children[active] as HTMLElement | undefined;
    cell?.scrollIntoView({ block: "center", behavior: "smooth" });
  }, [active]);

  return (
    <div className="wall" ref={gridRef}>
      {items.map((item, i) => (
        <div
          key={item.id}
          className={"cell" + (i === active ? " active" : "")}
          onClick={() => (i === active ? onApply(i) : onSelect(i))}
          onDoubleClick={() => onApply(i)}
          title={item.name}
        >
          <img src={thumbUrl(item)} alt="" loading="lazy" decoding="async" draggable={false} />
        </div>
      ))}
    </div>
  );
}
