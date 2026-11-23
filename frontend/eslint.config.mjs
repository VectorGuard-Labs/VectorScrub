import nextVitals from "eslint-config-next/core-web-vitals";
import nextTypescript from "eslint-config-next/typescript";

// eslint-config-next 16 ships flat configs, so no compatibility layer is needed.
const config = [
  { ignores: [".next/**", "node_modules/**", "next-env.d.ts", "**/*.generated.ts"] },
  ...nextVitals,
  ...nextTypescript,
];

export default config;
