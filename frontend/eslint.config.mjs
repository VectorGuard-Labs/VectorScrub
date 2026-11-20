import { FlatCompat } from "@eslint/eslintrc";
import { dirname } from "node:path";
import { fileURLToPath } from "node:url";

// eslint-config-next 15 is an eslintrc-style config; FlatCompat adapts it to ESLint 9's flat config.
const compat = new FlatCompat({ baseDirectory: dirname(fileURLToPath(import.meta.url)) });

const config = [
  { ignores: [".next/**", "node_modules/**", "next-env.d.ts", "**/*.generated.ts"] },
  ...compat.extends("next/core-web-vitals", "next/typescript"),
];

export default config;
