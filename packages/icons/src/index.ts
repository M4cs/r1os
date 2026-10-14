export { Icon, type IconProps } from "./Icon";
export { configureIcons, type IconSource, type IconWeight, iconSvgSync, loadIcon, preloadIcons, WEIGHTS } from "./icons";
export { type AppTile, appTile, type TileEffect } from "./tile";
export type { IconInfo } from "./catalog";
// The catalog is ≈ 450 KB: import `loadIconCatalog` from "@r1os/icons/catalog" where it's needed.
