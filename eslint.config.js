import js from '@eslint/js';
import tseslint from 'typescript-eslint';
import svelte from 'eslint-plugin-svelte';
import globals from 'globals';
import prettier from 'eslint-config-prettier';

export default tseslint.config(
  {
    ignores: [
      'dist/',
      'node_modules/',
      'src-tauri/target/',
      'src-tauri/gen/',
      'src-tauri/src/',
      // Android 构建产物与生成物（Capacitor 模板拷贝）：非外壳源码，不 lint。
      'mobile/android/build/',
      'mobile/android/.gradle/',
      'mobile/android/app/build/',
      'mobile/android/capacitor-cordova-android-plugins/',
      'mobile/node_modules/',
    ],
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  ...svelte.configs['flat/recommended'],
  prettier,
  ...svelte.configs['flat/prettier'],
  {
    languageOptions: {
      // 外壳跑在 WebView 里，浏览器全局是合法环境。
      globals: { ...globals.browser },
      parserOptions: {
        // <script lang="ts"> 与 *.svelte.ts 都交给 TS 解析器。
        parser: tseslint.parser,
        extraFileExtensions: ['.svelte'],
        sourceType: 'module',
      },
    },
  },
  {
    rules: {
      // 千寻规范 §3：不用无信息命名；§5：状态与展示分离。
      // ignoreRestSiblings：`const { kind, ...rest }` 的剥离式解构不算未使用。
      '@typescript-eslint/no-unused-vars': [
        'error',
        { argsIgnorePattern: '^_', ignoreRestSiblings: true },
      ],
      '@typescript-eslint/no-explicit-any': 'error',
    },
  },
);
