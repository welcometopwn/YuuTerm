/// <reference types="vite/client" />

/** Application version from package.json. */
declare const __YUUTERM_VERSION__: string;
/** "Release <commit>" from GitHub Actions, otherwise "Dev <commit>". */
declare const __YUUTERM_BUILD__: string;
/** "Dev <commit>" for builds made outside GitHub Actions, otherwise empty. */
declare const __YUUTERM_DEV_BUILD__: string;
