// Small pure helpers shared by the runtime (frame) and the shell-side bridge handlers. Kept free of json-render and
// React imports so the shell can use them without pulling the runtime's UI dependencies into its bundle.

/** `/q/<name>` with JSON-pointer escaping. */
export function queryPath(name: string): string {
  return "/q/" + name.replace(/~/g, "~0").replace(/\//g, "~1");
}

/** `:name` placeholders in a SQL string (ignores string literals and comments). */
export function sqlParamNames(sql: string): string[] {
  const names = new Set<string>();
  const stripped = sql.replace(/'(?:[^']|'')*'|--[^\n]*|\/\*[\s\S]*?\*\//g, " ");
  for (const m of stripped.matchAll(/(?<![:\w]):([A-Za-z_][A-Za-z0-9_]*)/g)) names.add(m[1] as string);
  return [...names];
}
