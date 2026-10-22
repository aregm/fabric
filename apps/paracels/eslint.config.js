// https://docs.expo.dev/guides/using-eslint/
const { defineConfig } = require('eslint/config');
const expoConfig = require('eslint-config-expo/flat');

module.exports = defineConfig([
  expoConfig,
  {
    ignores: ['dist/*'],
  },
  {
    // Metro bundles these files into the app, which runs in Hermes, not Node.
    files: ['src/**/*.{js,jsx,ts,tsx}'],
    ignores: ['src/app/api/**', 'src/chat/server/**'],
    rules: {
      'no-restricted-imports': [
        'error',
        {
          patterns: [
            {
              regex: '^node:',
              message: 'Node built-ins are available only to API routes and the socket server.',
            },
            {
              regex: '(^|/)chat/server(/|$)',
              message: 'src/chat/server/ is server-only; app code imports from src/chat/shared/.',
            },
          ],
        },
      ],
    },
  },
]);
