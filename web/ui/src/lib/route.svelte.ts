// Hash-based router for Svelte 5
// Parses location.hash (#/agents, #/agents/foo, #/history, etc.)
// Exports: router (class with reactive `current`), navigate() function

function parse(hash: string): { view: string; param: string | null } {
  const path = hash.replace(/^#\/?/, ""); // Remove "#/" prefix
  const [view = "agents", param = null] = path.split("/");
  return { view, param: param || null };
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
