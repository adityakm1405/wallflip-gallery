import { convertFileSrc, invoke } from "@tauri-apps/api/core";

export type StyleName = "carousel" | "depth" | "fan" | "wall";
export type StyleSetting = StyleName | "random";
export type CycleMode = "sequential" | "shuffle" | "norepeat";
export type Fit = "fill" | "fit" | "stretch" | "tile" | "center" | "span";

export type TransitionName =
  | "native" | "fade" | "slide" | "push" | "wipe" | "diagonal" | "circle" | "diamond"
  | "zoom" | "zoomout" | "blur" | "blinds" | "tiles" | "flip" | "random";

export interface Config {
  folder: string | null;
  recursive: boolean;
  style: StyleSetting;
  gallery_hotkey: string;
  cycle_hotkey: string;
  mode: CycleMode;
  fit: Fit;
  transition: TransitionName;
  transition_ms: number;
  interval_min: number;
  extensions: string[];
}

export interface Item {
  id: string;
  name: string;
}

export interface Gallery {
  items: Item[];
  /** id of the wallpaper currently on screen, if it is in the folder */
  current: string | null;
  folder: string | null;
  style: StyleSetting;
}

export const getGallery = () => invoke<Gallery>("get_gallery");
export const reloadGallery = () => invoke<Gallery>("reload_gallery");
export const applyWallpaper = (id: string) => invoke<void>("apply_wallpaper", { id });
export const hideOverlay = () => invoke<void>("hide_overlay");
export const openSettings = () => invoke<void>("open_settings_cmd");

export const getConfig = () => invoke<Config>("get_config");
export const saveConfig = (config: Config) => invoke<void>("save_config", { config });
export const previewTransition = (kind: TransitionName, ms: number) =>
  invoke<void>("preview_transition", { kind, ms });
export const chooseFolder = () => invoke<string | null>("choose_folder");
export const getAutostart = () => invoke<boolean>("get_autostart");
export const setAutostart = (enabled: boolean) => invoke<void>("set_autostart", { enabled });
export const quitApp = () => invoke<void>("quit_app");

/** URL served by the Rust `thumb` protocol (cached JPEG thumbnails). */
export const thumbUrl = (item: Item) => convertFileSrc(item.id, "thumb");
