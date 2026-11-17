#!/usr/bin/env node
/**
 * Fails when the web app starts loading anything from another company.
 *
 * The Privacy Policy says: no analytics, no session recording, no third-party trackers, and no
 * fonts, scripts or images from other companies. Each of those would also need the visitor's
 * consent (or at least a disclosure) before it loaded, and a font fetched from a font CDN sends
 * every visitor's IP address to that company. This check keeps the claim true: it reads the source,
 * the dependency list and the Content-Security-Policy, and exits non-zero on anything that
 * contradicts it. Run it with `npm run check:third-party` (CI does).
 *
 * To add a third-party service on purpose: get consent first (a banner that blocks the load until
 * accepted), update the Privacy Policy and bump TERMS_VERSION, then change this list.
 */
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, extname } from "node:path";

const root = new URL("..", import.meta.url).pathname;
const problems = [];

// Source patterns that load or configure a third-party resource.
const BANNED_SOURCE = [
  [/next\/font\/google/, "next/font/google contacts Google when the site is built and ties the build to their service; keep the font files in the repo and use next/font/local"],
  [/fonts\.(googleapis|gstatic)\.com/, "Google Fonts"],
  [/use\.typekit\.net|p\.typekit\.net/, "Adobe Fonts"],
  [/use\.fontawesome\.com|kit\.fontawesome\.com/, "Font Awesome CDN"],
  [/cdn\.jsdelivr\.net|cdnjs\.cloudflare\.com|unpkg\.com/, "a public CDN"],
  [/googletagmanager\.com|google-analytics\.com|gtag\s*\(/, "Google Analytics / Tag Manager"],
  [/hotjar\.com|static\.hotjar|fullstory\.com|logrocket|clarity\.ms|mouseflow|smartlook|rrweb|\bheap(analytics)?\.com|inspectlet|crazyegg|luckyorange/i, "session recording or replay"],
  [/posthog|mixpanel|amplitude\.com|segment\.(com|io)|plausible\.io|matomo|fathom/i, "product analytics"],
  [/@sentry\/|sentry\.io|datadoghq|newrelic|bugsnag|rollbar/i, "a third-party error tracker (it would receive page data)"],
  [/connect\.facebook\.net|fbq\s*\(|snap\.licdn\.com|analytics\.tiktok|static\.ads-twitter|doubleclick\.net/, "an advertising pixel"],
  [/intercom|drift\.com|crisp\.chat|hs-scripts|zdassets/i, "a third-party chat or support widget"],
  [/<script[^>]+src=["']https?:/i, "an external <script>"],
  [/<link[^>]+href=["']https?:[^>]*(stylesheet|preload|font)/i, "an external stylesheet or font"],
  [/@import\s+(url\()?["']?https?:/i, "an external CSS @import"],
  [/url\(\s*["']?https?:\/\//i, "an external CSS url()"],
];

const BANNED_PACKAGES = [
  /^@sentry\//, /^@datadog\//, /^posthog/, /^mixpanel/, /^@amplitude\//, /^analytics$/, /^@segment\//,
  /^logrocket/, /^@fullstory\//, /^react-ga/, /^@vercel\/analytics$/, /^@vercel\/speed-insights$/,
  /^hotjar/, /^@hotjar\//, /^rrweb/, /^@microsoft\/clarity$/, /^smartlook/, /^intercom/, /^@intercom\//,
  /^web-vitals$/, /^@next\/third-parties$/,
];

function* files(dir) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    const st = statSync(p);
    if (st.isDirectory()) {
      if (name === "node_modules" || name === ".next") continue;
      yield* files(p);
    } else if ([".ts", ".tsx", ".css", ".mjs", ".js", ".html"].includes(extname(p)) && !name.includes(".generated.")) {
      yield p;
    }
  }
}

function stripComments(text) {
  // Comments may name a vendor (for example "example: Sentry"); only code counts.
  return text.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:])\/\/[^\n]*/g, "$1");
}

for (const dir of ["src", "public"]) {
  let list;
  try { list = [...files(join(root, dir))]; } catch { continue; }
  for (const file of list) {
    if (file.endsWith("check-third-party.mjs")) continue;
    const code = stripComments(readFileSync(file, "utf8"));
    for (const [re, what] of BANNED_SOURCE) {
      if (re.test(code)) problems.push(`${file.replace(root, "")}: ${what}`);
    }
  }
}

// next.config: no image/remote hosts, no assetPrefix pointing elsewhere.
const config = stripComments(readFileSync(join(root, "next.config.ts"), "utf8"));
if (/remotePatterns|domains\s*:|assetPrefix/.test(config)) problems.push("next.config.ts: remote images or an asset prefix");

// Dependencies.
const pkg = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
for (const name of Object.keys({ ...pkg.dependencies, ...pkg.devDependencies })) {
  if (BANNED_PACKAGES.some((re) => re.test(name))) problems.push(`package.json: ${name}`);
}

// The Content-Security-Policy must stay closed to other origins for everything that loads.
const csp = stripComments(readFileSync(join(root, "src/lib/csp.ts"), "utf8"));
const directive = (name) => {
  const m = csp.match(new RegExp(`"${name}":\\s*\\[([^\\]]*)\\]`));
  return m ? m[1] : null;
};
for (const name of ["default-src", "script-src", "style-src", "img-src", "font-src", "connect-src"]) {
  const v = directive(name);
  if (v === null) problems.push(`csp.ts: no "${name}" directive`);
  else if (/https?:|\*/.test(v)) problems.push(`csp.ts: "${name}" allows another origin`);
}

if (problems.length) {
  console.error("Third-party content found. The Privacy Policy says there is none:\n");
  for (const p of problems) console.error("  - " + p);
  process.exit(1);
}
console.log("No third-party fonts, scripts, trackers or session recording found.");
