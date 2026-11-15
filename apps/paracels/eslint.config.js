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
    ignores: ['src/app/api/**', 'src/chat/server/**', 'src/**/*.test.ts'],
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
  {
    // The app, the API routes and the socket server all load these files.
    files: ['src/chat/shared/**/*.ts'],
    ignores: ['src/**/*.test.ts'],
    rules: {
      'no-restricted-imports': [
        'error',
        {
          patterns: [
            {
              regex: '^(?!\\./)',
              message: 'src/chat/shared/ imports nothing outside itself.',
            },
          ],
        },
      ],
    },
  },
  {
    // Plain Node loads these files. It strips types without resolving them, so a
    // type imported without `import type` fails at startup.
    files: ['src/chat/shared/**/*.ts', 'src/chat/server/**/*.ts', 'server/**/*.ts', 'scripts/**/*.ts'],
    rules: {
      '@typescript-eslint/consistent-type-imports': 'error',
    },
  },
  {
    files: ['src/app/api/**', 'src/chat/server/http.ts', 'src/chat/server/api-context.ts'],
    rules: {
      'no-restricted-imports': [
        'error',
        {
          patterns: [
            {
              regex: '(^|/)message-store(\\.ts)?$',
              importNames: ['createMessageWriter'],
              message: 'Only the socket server writes the messages table.',
            },
          ],
        },
      ],
    },
  },
  {
    files: ['server/**/*.ts'],
    ignores: ['server/**/*.test.ts'],
    rules: {
      'no-restricted-imports': [
        'error',
        {
          patterns: [
            {
              regex: '(^|/)attachment-store(\\.ts)?$',
              importNames: ['createAttachmentWriter'],
              message: 'Only the API routes write the attachments table.',
            },
          ],
        },
      ],
    },
  },
]);
