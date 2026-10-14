// List (contract §4): rows from an array, with Value IR row templates evaluated per row (`{i}` = row field) and
// row actions dispatched through json-render's ActionProvider as `run` (the same path as a Button's on.press).

import { useActions, useStateStore } from "@json-render/react";
import { memo, useCallback, useRef } from "react";

import { settle, useRuntime } from "../context";
import { eneeId } from "../spec";
import { evalValue, readPointer, toText, type ValueScope } from "../value";
import { Empty } from "./basic";
import { Icon } from "./icons";
import type { Ctx } from "./types";

interface RowAction {
  icon: string;
  label: string;
  action: string;
  args?: Record<string, unknown> | undefined;
}

type Row = Record<string, unknown>;

/**
 * A row thumbnail: one of the app's own files (clipboard history's images, served next to the app's document) or an
 * inline image. Anything else (a remote URL, a path) shows nothing; the frame's CSP would refuse it anyway.
 */
const THUMB_SRC = /^(?:files\/[0-9a-f]{64}(?:\.thumb)?\.png|data:image\/(?:png|jpeg|gif|webp);base64,[A-Za-z0-9+/=]+)$/;

export function thumbSrc(value: string): string {
  return THUMB_SRC.test(value) ? value : "";
}

function rowKeyOf(row: Row, itemKey: string, index: number): string {
  const v = row[itemKey];
  return v === undefined || v === null ? `#${index}` : String(v);
}

export function List({ props }: Ctx<"List">) {
  // Row templates may read state ({p}); subscribing keeps them live.
  const { state, getSnapshot } = useStateStore();
  const { execute } = useActions();
  const { format, reportError } = useRuntime();
  const items: Row[] = Array.isArray(props.items) ? props.items : [];

  // Row buttons are memoized; resolve the row and the action at click time from the latest render.
  const latest = useRef({ rows: new Map<string, Row>(), execute, getSnapshot, format, reportError });
  const rows = new Map<string, Row>();
  items.forEach((row, i) => rows.set(rowKeyOf(row, props.itemKey, i), row));
  latest.current = { rows, execute, getSnapshot, format, reportError };

  const onAction = useCallback((rowKey: string, ra: RowAction) => {
    const { rows, execute, getSnapshot, format, reportError } = latest.current;
    const scope: ValueScope = { read: (p) => readPointer(getSnapshot(), p), item: rows.get(rowKey), format };
    const args = evalValue(ra.args ?? {}, scope);
    settle(execute({ action: "run", params: { name: ra.action, args } }), reportError);
  }, []);

  if (items.length === 0) {
    return (
      <div className="enee-list" data-empty="true" {...eneeId(props)}>
        <Empty text={props.empty ?? "Nothing here yet"} />
      </div>
    );
  }

  const read = (path: string) => readPointer(state, path);
  return (
    <div className="enee-list" role="list" {...eneeId(props)}>
      {items.map((row, index) => {
        const scope: ValueScope = { read, item: row, format };
        const key = rowKeyOf(row, props.itemKey, index);
        return (
          <ListRow
            key={key}
            rowKey={key}
            title={toText(evalValue(props.title, scope))}
            caption={props.caption === undefined ? "" : toText(evalValue(props.caption, scope))}
            image={props.image === undefined ? "" : thumbSrc(toText(evalValue(props.image, scope)))}
            actions={props.rowActions}
            onAction={onAction}
          />
        );
      })}
    </div>
  );
}

const ListRow = memo(function ListRow({
  rowKey,
  title,
  caption,
  image,
  actions,
  onAction,
}: {
  rowKey: string;
  title: string;
  caption: string;
  image: string;
  actions: RowAction[] | undefined;
  onAction: (rowKey: string, action: RowAction) => void;
}) {
  return (
    <div className="enee-list__row" role="listitem" data-enee-row={rowKey}>
      {image ? <img className="enee-list__thumb" src={image} alt="" loading="lazy" decoding="async" draggable={false} /> : null}
      <div className="enee-list__text">
        <div className="enee-list__title">{title}</div>
        {caption ? <div className="enee-list__caption">{caption}</div> : null}
      </div>
      {actions && actions.length > 0 ? (
        <div className="enee-list__actions">
          {actions.map((ra, i) => (
            <button
              key={i}
              type="button"
              className="enee-icon-button"
              data-icon={ra.icon}
              aria-label={ra.label}
              title={ra.label}
              onClick={() => onAction(rowKey, ra)}
            >
              <Icon name={ra.icon} size={15} />
            </button>
          ))}
        </div>
      ) : null}
    </div>
  );
});
