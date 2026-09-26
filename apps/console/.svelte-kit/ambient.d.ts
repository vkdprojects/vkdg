
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
	export const NODE_ENV: string;
	export const COLORTERM: string;
	export const npm_node_execpath: string;
	export const GIT_PAGER: string;
	export const OSLogRateLimit: string;
	export const PROXMOX_TOKEN: string;
	export const SEARXNG_BASIC_PASSWORD: string;
	export const MANPAGER: string;
	export const CANOPYWAVE_API_KEY: string;
	export const AGENT: string;
	export const npm_config_user_agent: string;
	export const BUN_INSTALL: string;
	export const XDG_DATA_DIRS: string;
	export const npm_package_json: string;
	export const npm_lifecycle_script: string;
	export const GHOSTTY_BIN_DIR: string;
	export const MYSQL_PAGER: string;
	export const USER: string;
	export const FNM_DIR: string;
	export const GH_PAGER: string;
	export const SHLVL: string;
	export const ORCA_PI_STATUS_OWNED: string;
	export const GPG_TTY: string;
	export const npm_config_yes: string;
	export const SSH_ASKPASS: string;
	export const STARSHIP_SHELL: string;
	export const GHOSTTY_SHELL_FEATURES: string;
	export const npm_package_version: string;
	export const FNM_MULTISHELL_PATH: string;
	export const LANG: string;
	export const npm_package_name: string;
	export const EDITOR: string;
	export const npm_lifecycle_event: string;
	export const PIP_DISABLE_PIP_VERSION_CHECK: string;
	export const YARN_ENABLE_TELEMETRY: string;
	export const DELTA_PAGER: string;
	export const LESS: string;
	export const BAT_PAGER: string;
	export const JAVA_HOME: string;
	export const GH_PROMPT_DISABLED: string;
	export const PWD: string;
	export const GIT_EDITOR: string;
	export const PNPM_DISABLE_SELF_UPDATE_CHECK: string;
	export const __CFBundleIdentifier: string;
	export const _: string;
	export const STARSHIP_SESSION_KEY: string;
	export const COMPOSER_NO_INTERACTION: string;
	export const XPC_SERVICE_NAME: string;
	export const FNM_LOGLEVEL: string;
	export const PROXMOX_HOST: string;
	export const CLOUDSDK_CORE_DISABLE_PROMPTS: string;
	export const ANDROID_HOME: string;
	export const PAGER: string;
	export const FNM_ARCH: string;
	export const MANPATH: string;
	export const TF_INPUT: string;
	export const NO_COLOR: string;
	export const SYSTEMD_PAGER: string;
	export const npm_execpath: string;
	export const COMMAND_MODE: string;
	export const DEBIAN_FRONTEND: string;
	export const PSQL_PAGER: string;
	export const GIT_TERMINAL_PROMPT: string;
	export const SVELTEKIT_FORK: string;
	export const HOMEBREW_PAGER: string;
	export const FNM_RESOLVE_ENGINES: string;
	export const SSH_AUTH_SOCK: string;
	export const TERMINFO: string;
	export const CLAUDECODE: string;
	export const ANTHROPIC_API_KEY: string;
	export const XPC_FLAGS: string;
	export const npm_command: string;
	export const FNM_VERSION_FILE_STRATEGY: string;
	export const FNM_COREPACK_ENABLED: string;
	export const CARGO_TERM_PROGRESS_WHEN: string;
	export const AWS_PAGER: string;
	export const LOGNAME: string;
	export const ANTHROPIC_BASE_URL: string;
	export const GLAB_PAGER: string;
	export const npm_config_local_prefix: string;
	export const __CF_USER_TEXT_ENCODING: string;
	export const VISUAL: string;
	export const PNPM_UPDATE_NOTIFIER: string;
	export const PIP_NO_INPUT: string;
	export const GHOSTTY_RESOURCES_DIR: string;
	export const npm_config_progress: string;
	export const TERM: string;
	export const TMPDIR: string;
	export const CI: string;
	export const PYTHONUNBUFFERED: string;
	export const PATH: string;
	export const npm_config_update_notifier: string;
	export const ANTHROPIC_SEARCH_BASE_URL: string;
	export const YARN_ENABLE_PROGRESS_BARS: string;
	export const SEARXNG_BASIC_USERNAME: string;
	export const NODE: string;
	export const TF_IN_AUTOMATION: string;
	export const FNM_NODE_DIST_MIRROR: string;
	export const ANTHROPIC_SEARCH_API_KEY: string;
	export const HOME: string;
	export const TERM_PROGRAM: string;
	export const npm_config_fund: string;
	export const npm_config_audit: string;
	export const TERM_PROGRAM_VERSION: string;
	export const OMPCODE: string;
	export const CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY: string;
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
		NODE_ENV: string;
		COLORTERM: string;
		npm_node_execpath: string;
		GIT_PAGER: string;
		OSLogRateLimit: string;
		PROXMOX_TOKEN: string;
		SEARXNG_BASIC_PASSWORD: string;
		MANPAGER: string;
		CANOPYWAVE_API_KEY: string;
		AGENT: string;
		npm_config_user_agent: string;
		BUN_INSTALL: string;
		XDG_DATA_DIRS: string;
		npm_package_json: string;
		npm_lifecycle_script: string;
		GHOSTTY_BIN_DIR: string;
		MYSQL_PAGER: string;
		USER: string;
		FNM_DIR: string;
		GH_PAGER: string;
		SHLVL: string;
		ORCA_PI_STATUS_OWNED: string;
		GPG_TTY: string;
		npm_config_yes: string;
		SSH_ASKPASS: string;
		STARSHIP_SHELL: string;
		GHOSTTY_SHELL_FEATURES: string;
		npm_package_version: string;
		FNM_MULTISHELL_PATH: string;
		LANG: string;
		npm_package_name: string;
		EDITOR: string;
		npm_lifecycle_event: string;
		PIP_DISABLE_PIP_VERSION_CHECK: string;
		YARN_ENABLE_TELEMETRY: string;
		DELTA_PAGER: string;
		LESS: string;
		BAT_PAGER: string;
		JAVA_HOME: string;
		GH_PROMPT_DISABLED: string;
		PWD: string;
		GIT_EDITOR: string;
		PNPM_DISABLE_SELF_UPDATE_CHECK: string;
		__CFBundleIdentifier: string;
		_: string;
		STARSHIP_SESSION_KEY: string;
		COMPOSER_NO_INTERACTION: string;
		XPC_SERVICE_NAME: string;
		FNM_LOGLEVEL: string;
		PROXMOX_HOST: string;
		CLOUDSDK_CORE_DISABLE_PROMPTS: string;
		ANDROID_HOME: string;
		PAGER: string;
		FNM_ARCH: string;
		MANPATH: string;
		TF_INPUT: string;
		NO_COLOR: string;
		SYSTEMD_PAGER: string;
		npm_execpath: string;
		COMMAND_MODE: string;
		DEBIAN_FRONTEND: string;
		PSQL_PAGER: string;
		GIT_TERMINAL_PROMPT: string;
		SVELTEKIT_FORK: string;
		HOMEBREW_PAGER: string;
		FNM_RESOLVE_ENGINES: string;
		SSH_AUTH_SOCK: string;
		TERMINFO: string;
		CLAUDECODE: string;
		ANTHROPIC_API_KEY: string;
		XPC_FLAGS: string;
		npm_command: string;
		FNM_VERSION_FILE_STRATEGY: string;
		FNM_COREPACK_ENABLED: string;
		CARGO_TERM_PROGRESS_WHEN: string;
		AWS_PAGER: string;
		LOGNAME: string;
		ANTHROPIC_BASE_URL: string;
		GLAB_PAGER: string;
		npm_config_local_prefix: string;
		__CF_USER_TEXT_ENCODING: string;
		VISUAL: string;
		PNPM_UPDATE_NOTIFIER: string;
		PIP_NO_INPUT: string;
		GHOSTTY_RESOURCES_DIR: string;
		npm_config_progress: string;
		TERM: string;
		TMPDIR: string;
		CI: string;
		PYTHONUNBUFFERED: string;
		PATH: string;
		npm_config_update_notifier: string;
		ANTHROPIC_SEARCH_BASE_URL: string;
		YARN_ENABLE_PROGRESS_BARS: string;
		SEARXNG_BASIC_USERNAME: string;
		NODE: string;
		TF_IN_AUTOMATION: string;
		FNM_NODE_DIST_MIRROR: string;
		ANTHROPIC_SEARCH_API_KEY: string;
		HOME: string;
		TERM_PROGRAM: string;
		npm_config_fund: string;
		npm_config_audit: string;
		TERM_PROGRAM_VERSION: string;
		OMPCODE: string;
		CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY: string;
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
