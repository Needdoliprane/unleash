import js from "@eslint/js";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";
import jsxA11y from "eslint-plugin-jsx-a11y";

export default tseslint.config(
  {
    ignores: [
      "dist/**",
      "node_modules/**",
      "src/generated/**",
      "src/generated-gui/**",
      "vite.config.ts",
      "eslint.config.js",
    ],
  },
  js.configs.recommended,
  ...tseslint.configs.strictTypeChecked,
  jsxA11y.flatConfigs.recommended,
  {
    languageOptions: {
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
      },
    },
    plugins: {
      "react-hooks": reactHooks,
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      "@typescript-eslint/no-explicit-any": "error",
      "@typescript-eslint/no-unsafe-assignment": "error",
      "@typescript-eslint/no-unsafe-call": "error",
      "@typescript-eslint/no-unsafe-member-access": "error",
      "@typescript-eslint/no-unsafe-return": "error",
      "no-eval": "error",
      "no-implied-eval": "error",
      "no-new-func": "error",
      "no-restricted-properties": [
        "error",
        {
          property: "innerHTML",
          message: "innerHTML est interdit : rendu React échappé uniquement (ADR-03).",
        },
        {
          property: "outerHTML",
          message: "outerHTML est interdit : rendu React échappé uniquement (ADR-03).",
        },
        {
          property: "insertAdjacentHTML",
          message: "insertAdjacentHTML est interdit (ADR-03).",
        },
        {
          object: "document",
          property: "write",
          message: "document.write est interdit (ADR-03).",
        },
      ],
      "no-restricted-syntax": [
        "error",
        {
          selector: "JSXAttribute[name.name='dangerouslySetInnerHTML']",
          message: "dangerouslySetInnerHTML est strictement interdit (ADR-03 / REQ-UI-014).",
        },
        {
          selector: "Identifier[name='dangerouslySetInnerHTML']",
          message: "dangerouslySetInnerHTML est strictement interdit (ADR-03 / REQ-UI-014).",
        },
      ],
    },
  },
);
