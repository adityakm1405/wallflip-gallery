import { memo, useState } from "react";
import type { CSSProperties } from "react";
import { thumbUrl } from "../lib/api";
import type { Item } from "../lib/api";

interface Props {
  item: Item;
  active: boolean;
  style: CSSProperties;
  dim: number;
  onClick: (active: boolean) => void;
  onDoubleClick: () => void;
}

/** One gallery card. Memoised so only cards whose look changed re-render. */
export const Card = memo(function Card({ item, active, style, dim, onClick, onDoubleClick }: Props) {
  const [loaded, setLoaded] = useState(false);
  return (
    <div
      className={"card" + (active ? " active" : "")}
      style={style}
      onClick={() => onClick(active)}
      onDoubleClick={onDoubleClick}
    >
      <img
        src={thumbUrl(item)}
        alt=""
        draggable={false}
        decoding="async"
        className={loaded ? "loaded" : ""}
        onLoad={() => setLoaded(true)}
      />
      <div className="shade" style={{ opacity: dim }} />
    </div>
  );
});
