import { defineConfig, globalIgnores } from 'eslint/config';
import globals from 'globals';
import js from '@eslint/js';
import tseslint from 'typescript-eslint';
import react from 'eslint-plugin-react';
import reactHooks from 'eslint-plugin-react-hooks';
import prettier from 'eslint-config-prettier';

// Flat config in TypeScript syntax (requires `jiti` to load `eslint.config.ts`).
// Run with: pnpm lint
export default defineConfig([
  // Ignore build output, deps, Tauri/Rust artefacts, generated workers, logs
  globalIgnores([
    'dist/',
    'dist-ssr/',
    'node_modules/',
    'src-tauri/target/',
    'src-tauri/gen/',
    'embed-worker/',
    '*.log',
    '*.local',
  ]),

  // Base JS + TS rules
  js.configs.recommended,
  ...tseslint.configs.recommended,

  // React 19 with the new JSX transform (`"jsx": "react-jsx"` in tsconfig).
  // `jsx-runtime` disables `react/jsx-uses-react` / `react/react-in-jsx-scope`.
  react.configs.flat.recommended,
  react.configs.flat['jsx-runtime'],
  reactHooks.configs.flat['recommended-latest'],

  // Must stay last: turns off ESLint stylistic rules that conflict with Prettier.
  // Formatting itself is done via `pnpm format` (Prettier), not via ESLint.
  prettier,

  // Project-specific tweaks
  {
    languageOptions: {
      globals: {
        ...globals.browser,
        ...globals.node,
      },
    },
    settings: {
      react: { version: 'detect' },
    },
    linterOptions: {
      reportUnusedDisableDirectives: 'error',
    },
    rules: {
      eqeqeq: 'error',
      // 'multi-line' allows single-line guards (`if (x) return;`) but still
      // requires braces for multi-line blocks. Tighten to 'all' later.
      curly: ['error', 'multi-line'],
      'no-unreachable': 'error',
      'no-console': ['warn', { allow: ['warn', 'error'] }],
      // Warn (not error) for now: the codebase uses `any` in several
      // handlers (`friendlyError(err: any)`). Fix progressively.
      '@typescript-eslint/no-explicit-any': 'warn',
    },
  },
]);
