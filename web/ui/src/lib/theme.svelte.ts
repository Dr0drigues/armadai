class Theme {
  value = $state<"dark" | "light">("dark");

  toggle() {
    this.value = this.value === "dark" ? "light" : "dark";
    document.documentElement.setAttribute("data-theme", this.value);
  }

  init() {
    // On mount, read from DOM or system preference
    const stored = document.documentElement.getAttribute("data-theme");
    if (stored === "light" || stored === "dark") {
      this.value = stored;
    } else if (matchMedia("(prefers-color-scheme: light)").matches) {
      this.value = "light";
    }
  }
}

export const theme = new Theme();
