// The tagged Phosphor catalog (≈ 450 KB). Import it lazily (`await loadIconCatalog()`): pickers and search need it,
// rendering an icon doesn't.

export interface IconInfo {
  name: string;
  tags: readonly string[];
  categories: readonly string[];
}

let catalog: Promise<IconInfo[]> | null = null;

export function loadIconCatalog(): Promise<IconInfo[]> {
  catalog ??= import("@phosphor-icons/core").then((m) =>
    m.icons.map((i) => ({ name: i.name, tags: i.tags, categories: i.categories })),
  );
  return catalog;
}
