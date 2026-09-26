
// this file is generated — do not edit it


/// <reference types="@sveltejs/kit" />

/**
 * This module provides access to environment variables that are injected _statically_ into your bundle at build time and are limited to _private_ access.
 * 
 * |         | Runtime                                                                    | Build time                                                               |
 * | ------- | -------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
 * | Private | [`$env/dynamic/private`](https://svelte.dev/docs/kit/$env-dynamic-private) | [`$env/static/private`](https://svelte.dev/docs/kit/$env-static-private) |
 * | Public  | [`$env/dynamic/public`](https://svelte.dev/docs/kit/$env-dynamic-public)   | [`$env/static/public`](https://svelte.dev/docs/kit/$env-static-public)   |
 * 
 * Static environment variables are [loaded by Vite](https://vitejs.dev/guide/env-and-mode.html#env-files) from `.env` files and `process.env` at build time and then statically injected into your bundle at build time, enabling optimisations like dead code elimination.
 * 
 * **_Private_ access:**
 * 
 * - This module cannot be imported into client-side code
 * - This module only includes variables that _do not_ begin with [`config.kit.env.publicPrefix`](https://svelte.dev/docs/kit/configuration#env) _and do_ start with [`config.kit.env.privatePrefix`](https://svelte.dev/docs/kit/configuration#env) (if configured)
 * 
 * For example, given the following build time environment:
 * 
 * ```env
 * ENVIRONMENT=production
 * PUBLIC_BASE_URL=http://site.com
 * ```
 * 
 * With the default `publicPrefix` and `privatePrefix`:
 * 
 * ```ts
 * import { ENVIRONMENT, PUBLIC_BASE_URL } from '$env/static/private';
 * 
 * console.log(ENVIRONMENT); // => "production"
 * console.log(PUBLIC_BASE_URL); // => throws error during build
 * ```
 * 
 * The above values will be the same _even if_ different values for `ENVIRONMENT` or `PUBLIC_BASE_URL` are set at runtime, as they are statically replaced in your code with their build time values.
 */
declare module '$env/static/private' {
	export const AGENT: string;
	export const ANDROID_HOME: string;
	export const ANTHROPIC_API_KEY: string;
	export const ANTHROPIC_BASE_URL: string;
	export const ANTHROPIC_SEARCH_API_KEY: string;
	export const ANTHROPIC_SEARCH_BASE_URL: string;
	export const AWS_PAGER: string;
	export const BAT_PAGER: string;
	export const BUN_INSTALL: string;
	export const CANOPYWAVE_API_KEY: string;
	export const CARGO_TERM_PROGRESS_WHEN: string;
	export const CI: string;
	export const CLAUDECODE: string;
	export const CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY: string;
	export const CLOUDSDK_CORE_DISABLE_PROMPTS: string;
	export const COLORTERM: string;
	export const COMMAND_MODE: string;
	export const COMPOSER_NO_INTERACTION: string;
	export const DEBIAN_FRONTEND: string;
	export const DELTA_PAGER: string;
	export const EDITOR: string;
	export const FNM_ARCH: string;
	export const FNM_COREPACK_ENABLED: string;
	export const FNM_DIR: string;
	export const FNM_LOGLEVEL: string;
	export const FNM_MULTISHELL_PATH: string;
	export const FNM_NODE_DIST_MIRROR: string;
	export const FNM_RESOLVE_ENGINES: string;
	export const FNM_VERSION_FILE_STRATEGY: string;
	export const GHOSTTY_BIN_DIR: string;
	export const GHOSTTY_RESOURCES_DIR: string;
	export const GHOSTTY_SHELL_FEATURES: string;
	export const GH_PAGER: string;
	export const GH_PROMPT_DISABLED: string;
	export const GIT_EDITOR: string;
	export const GIT_PAGER: string;
	export const GIT_TERMINAL_PROMPT: string;
	export const GLAB_PAGER: string;
	export const GPG_TTY: string;
	export const HOME: string;
	export const HOMEBREW_PAGER: string;
	export const JAVA_HOME: string;
	export const LANG: string;
	export const LESS: string;
	export const LOGNAME: string;
	export const MANPAGER: string;
	export const MANPATH: string;
	export const MYSQL_PAGER: string;
	export const NO_COLOR: string;
	export const OLDPWD: string;
	export const OMPCODE: string;
	export const ORCA_PI_STATUS_OWNED: string;
	export const OSLogRateLimit: string;
	export const PAGER: string;
	export const PATH: string;
	export const PIP_DISABLE_PIP_VERSION_CHECK: string;
	export const PIP_NO_INPUT: string;
	export const PNPM_DISABLE_SELF_UPDATE_CHECK: string;
	export const PNPM_UPDATE_NOTIFIER: string;
	export const PROXMOX_HOST: string;
	export const PROXMOX_TOKEN: string;
	export const PSQL_PAGER: string;
	export const PWD: string;
	export const PYTHONUNBUFFERED: string;
	export const SEARXNG_BASIC_PASSWORD: string;
	export const SEARXNG_BASIC_USERNAME: string;
	export const SHLVL: string;
	export const SSH_ASKPASS: string;
	export const SSH_AUTH_SOCK: string;
	export const STARSHIP_SESSION_KEY: string;
	export const STARSHIP_SHELL: string;
	export const SYSTEMD_PAGER: string;
	export const TERM: string;
	export const TERMINFO: string;
	export const TERM_PROGRAM: string;
	export const TERM_PROGRAM_VERSION: string;
	export const TF_INPUT: string;
	export const TF_IN_AUTOMATION: string;
	export const TMPDIR: string;
	export const USER: string;
	export const VISUAL: string;
	export const XDG_DATA_DIRS: string;
	export const XPC_FLAGS: string;
	export const XPC_SERVICE_NAME: string;
	export const YARN_ENABLE_PROGRESS_BARS: string;
	export const YARN_ENABLE_TELEMETRY: string;
	export const _: string;
	export const __CFBundleIdentifier: string;
	export const __CF_USER_TEXT_ENCODING: string;
	export const npm_config_audit: string;
	export const npm_config_fund: string;
	export const npm_config_progress: string;
	export const npm_config_update_notifier: string;
	export const npm_config_yes: string;
	export const NODE_ENV: string;
}

/**
 * This module provides access to environment variables that are injected _statically_ into your bundle at build time and are _publicly_ accessible.
 * 
 * |         | Runtime                                                                    | Build time                                                               |
 * | ------- | -------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
 * | Private | [`$env/dynamic/private`](https://svelte.dev/docs/kit/$env-dynamic-private) | [`$env/static/private`](https://svelte.dev/docs/kit/$env-static-private) |
 * | Public  | [`$env/dynamic/public`](https://svelte.dev/docs/kit/$env-dynamic-public)   | [`$env/static/public`](https://svelte.dev/docs/kit/$env-static-public)   |
 * 
 * Static environment variables are [loaded by Vite](https://vitejs.dev/guide/env-and-mode.html#env-files) from `.env` files and `process.env` at build time and then statically injected into your bundle at build time, enabling optimisations like dead code elimination.
 * 
 * **_Public_ access:**
 * 
 * - This module _can_ be imported into client-side code
 * - **Only** variables that begin with [`config.kit.env.publicPrefix`](https://svelte.dev/docs/kit/configuration#env) (which defaults to `PUBLIC_`) are included
 * 
 * For example, given the following build time environment:
 * 
 * ```env
 * ENVIRONMENT=production
 * PUBLIC_BASE_URL=http://site.com
 * ```
 * 
 * With the default `publicPrefix` and `privatePrefix`:
 * 
 * ```ts
 * import { ENVIRONMENT, PUBLIC_BASE_URL } from '$env/static/public';
 * 
 * console.log(ENVIRONMENT); // => throws error during build
 * console.log(PUBLIC_BASE_URL); // => "http://site.com"
 * ```
 * 
 * The above values will be the same _even if_ different values for `ENVIRONMENT` or `PUBLIC_BASE_URL` are set at runtime, as they are statically replaced in your code with their build time values.
 */
declare module '$env/static/public' {
	
}

/**
 * This module provides access to environment variables set _dynamically_ at runtime and that are limited to _private_ access.
 * 
 * |         | Runtime                                                                    | Build time                                                               |
 * | ------- | -------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
 * | Private | [`$env/dynamic/private`](https://svelte.dev/docs/kit/$env-dynamic-private) | [`$env/static/private`](https://svelte.dev/docs/kit/$env-static-private) |
 * | Public  | [`$env/dynamic/public`](https://svelte.dev/docs/kit/$env-dynamic-public)   | [`$env/static/public`](https://svelte.dev/docs/kit/$env-static-public)   |
 * 
 * Dynamic environment variables are defined by the platform you're running on. For example if you're using [`adapter-node`](https://github.com/sveltejs/kit/tree/main/packages/adapter-node) (or running [`vite preview`](https://svelte.dev/docs/kit/cli)), this is equivalent to `process.env`.
 * 
 * **_Private_ access:**
 * 
 * - This module cannot be imported into client-side code
 * - This module includes variables that _do not_ begin with [`config.kit.env.publicPrefix`](https://svelte.dev/docs/kit/configuration#env) _and do_ start with [`config.kit.env.privatePrefix`](https://svelte.dev/docs/kit/configuration#env) (if configured)
 * 
 * > [!NOTE] In `dev`, `$env/dynamic` includes environment variables from `.env`. In `prod`, this behavior will depend on your adapter.
 * 
 * > [!NOTE] To get correct types, environment variables referenced in your code should be declared (for example in an `.env` file), even if they don't have a value until the app is deployed:
 * >
 * > ```env
 * > MY_FEATURE_FLAG=
 * > ```
 * >
 * > You can override `.env` values from the command line like so:
 * >
 * > ```sh
 * > MY_FEATURE_FLAG="enabled" npm run dev
 * > ```
 * 
 * For example, given the following runtime environment:
 * 
 * ```env
 * ENVIRONMENT=production
 * PUBLIC_BASE_URL=http://site.com
 * ```
 * 
 * With the default `publicPrefix` and `privatePrefix`:
 * 
 * ```ts
 * import { env } from '$env/dynamic/private';
 * 
 * console.log(env.ENVIRONMENT); // => "production"
 * console.log(env.PUBLIC_BASE_URL); // => undefined
 * ```
 */
declare module '$env/dynamic/private' {
	export const env: {
		AGENT: string;
		ANDROID_HOME: string;
		ANTHROPIC_API_KEY: string;
		ANTHROPIC_BASE_URL: string;
		ANTHROPIC_SEARCH_API_KEY: string;
		ANTHROPIC_SEARCH_BASE_URL: string;
		AWS_PAGER: string;
		BAT_PAGER: string;
		BUN_INSTALL: string;
		CANOPYWAVE_API_KEY: string;
		CARGO_TERM_PROGRESS_WHEN: string;
		CI: string;
		CLAUDECODE: string;
		CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY: string;
		CLOUDSDK_CORE_DISABLE_PROMPTS: string;
		COLORTERM: string;
		COMMAND_MODE: string;
		COMPOSER_NO_INTERACTION: string;
		DEBIAN_FRONTEND: string;
		DELTA_PAGER: string;
		EDITOR: string;
		FNM_ARCH: string;
		FNM_COREPACK_ENABLED: string;
		FNM_DIR: string;
		FNM_LOGLEVEL: string;
		FNM_MULTISHELL_PATH: string;
		FNM_NODE_DIST_MIRROR: string;
		FNM_RESOLVE_ENGINES: string;
		FNM_VERSION_FILE_STRATEGY: string;
		GHOSTTY_BIN_DIR: string;
		GHOSTTY_RESOURCES_DIR: string;
		GHOSTTY_SHELL_FEATURES: string;
		GH_PAGER: string;
		GH_PROMPT_DISABLED: string;
		GIT_EDITOR: string;
		GIT_PAGER: string;
		GIT_TERMINAL_PROMPT: string;
		GLAB_PAGER: string;
		GPG_TTY: string;
		HOME: string;
		HOMEBREW_PAGER: string;
		JAVA_HOME: string;
		LANG: string;
		LESS: string;
		LOGNAME: string;
		MANPAGER: string;
		MANPATH: string;
		MYSQL_PAGER: string;
		NO_COLOR: string;
		OLDPWD: string;
		OMPCODE: string;
		ORCA_PI_STATUS_OWNED: string;
		OSLogRateLimit: string;
		PAGER: string;
		PATH: string;
		PIP_DISABLE_PIP_VERSION_CHECK: string;
		PIP_NO_INPUT: string;
		PNPM_DISABLE_SELF_UPDATE_CHECK: string;
		PNPM_UPDATE_NOTIFIER: string;
		PROXMOX_HOST: string;
		PROXMOX_TOKEN: string;
		PSQL_PAGER: string;
		PWD: string;
		PYTHONUNBUFFERED: string;
		SEARXNG_BASIC_PASSWORD: string;
		SEARXNG_BASIC_USERNAME: string;
		SHLVL: string;
		SSH_ASKPASS: string;
		SSH_AUTH_SOCK: string;
		STARSHIP_SESSION_KEY: string;
		STARSHIP_SHELL: string;
		SYSTEMD_PAGER: string;
		TERM: string;
		TERMINFO: string;
		TERM_PROGRAM: string;
		TERM_PROGRAM_VERSION: string;
		TF_INPUT: string;
		TF_IN_AUTOMATION: string;
		TMPDIR: string;
		USER: string;
		VISUAL: string;
		XDG_DATA_DIRS: string;
		XPC_FLAGS: string;
		XPC_SERVICE_NAME: string;
		YARN_ENABLE_PROGRESS_BARS: string;
		YARN_ENABLE_TELEMETRY: string;
		_: string;
		__CFBundleIdentifier: string;
		__CF_USER_TEXT_ENCODING: string;
		npm_config_audit: string;
		npm_config_fund: string;
		npm_config_progress: string;
		npm_config_update_notifier: string;
		npm_config_yes: string;
		NODE_ENV: string;
		[key: `PUBLIC_${string}`]: undefined;
		[key: `${string}`]: string | undefined;
	}
}

/**
 * This module provides access to environment variables set _dynamically_ at runtime and that are _publicly_ accessible.
 * 
 * |         | Runtime                                                                    | Build time                                                               |
 * | ------- | -------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
 * | Private | [`$env/dynamic/private`](https://svelte.dev/docs/kit/$env-dynamic-private) | [`$env/static/private`](https://svelte.dev/docs/kit/$env-static-private) |
 * | Public  | [`$env/dynamic/public`](https://svelte.dev/docs/kit/$env-dynamic-public)   | [`$env/static/public`](https://svelte.dev/docs/kit/$env-static-public)   |
 * 
 * Dynamic environment variables are defined by the platform you're running on. For example if you're using [`adapter-node`](https://github.com/sveltejs/kit/tree/main/packages/adapter-node) (or running [`vite preview`](https://svelte.dev/docs/kit/cli)), this is equivalent to `process.env`.
 * 
 * **_Public_ access:**
 * 
 * - This module _can_ be imported into client-side code
 * - **Only** variables that begin with [`config.kit.env.publicPrefix`](https://svelte.dev/docs/kit/configuration#env) (which defaults to `PUBLIC_`) are included
 * 
 * > [!NOTE] In `dev`, `$env/dynamic` includes environment variables from `.env`. In `prod`, this behavior will depend on your adapter.
 * 
 * > [!NOTE] To get correct types, environment variables referenced in your code should be declared (for example in an `.env` file), even if they don't have a value until the app is deployed:
 * >
 * > ```env
 * > MY_FEATURE_FLAG=
 * > ```
 * >
 * > You can override `.env` values from the command line like so:
 * >
 * > ```sh
 * > MY_FEATURE_FLAG="enabled" npm run dev
 * > ```
 * 
 * For example, given the following runtime environment:
 * 
 * ```env
 * ENVIRONMENT=production
 * PUBLIC_BASE_URL=http://example.com
 * ```
 * 
 * With the default `publicPrefix` and `privatePrefix`:
 * 
 * ```ts
 * import { env } from '$env/dynamic/public';
 * console.log(env.ENVIRONMENT); // => undefined, not public
 * console.log(env.PUBLIC_BASE_URL); // => "http://example.com"
 * ```
 * 
 * ```
 * 
 * ```
 */
declare module '$env/dynamic/public' {
	export const env: {
		[key: `PUBLIC_${string}`]: string | undefined;
	}
}
