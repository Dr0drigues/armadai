// Hash-based router for Svelte 5
// Parses location.hash (#/agents, #/agents/foo, #/history, etc.)
// Exports: router (class with reactive `current`), navigate() function

function parse(hash: string): { view: string; param: string | null } {
  const path = hash.replace(/^#\/?/, ""); // Remove "#/" prefix
  const [rawView, rawParam] = path.split("/");
  // Empty hash (#, #/, or no hash) → default view. Note a "" segment is not
  // `undefined`, so a destructuring default wouldn't catch it.
  const view = rawView || "agents";
  // Decode the param: navigate() encodes it (encodeURIComponent), and names
  // can contain spaces/slashes ("Dev Lead" → "Dev%20Lead").
  const param = rawParam ? decodeURIComponent(rawParam) : null;
  return { view, param };
}

class Router {
  current = $state(parse(location.hash));

  constructor() {
    addEventListener("hashchange", () => {
      this.current = parse(location.hash);
    });
  }

  navigate(path: string) {
    location.hash = "#/" + path.replace(/^#?\/?/, "");
  }
}

export const router = new Router();
export const navigate = (p: string) => router.navigate(p);
