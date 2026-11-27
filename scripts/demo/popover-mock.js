// Mounts the real popover (ui/index.html) in an iframe and feeds it the
// generated demo data through a stubbed window.__TAURI__.
(() => {
  const data = window.LimitlaneDemoData;
  const DARK_QUERY = "@media (prefers-color-scheme: dark)";

  function stubTauri(five, week) {
    return {
      core: {
        invoke: async (cmd, args) => {
          switch (cmd) {
            case "get_summary": return data.summary(args.rangeDays, five, week);
            case "app_info": return { version: "0.1.0", dev_build: false, launch_at_login: true, platform: "macos" };
            case "get_config": return { thresholds: [50, 75, 90], forecast_alerts: true, notifications: true, refresh_secs: 30, menu_bar: "both", pricing_overrides: [] };
            default: return null;
          }
        },
      },
      event: { listen: async () => () => {} },
    };
  }

  /**
   * `scheme` forces "light" or "dark" for this popover only; leave it out to
   * follow the page. Resolves with the popover's document once it rendered.
   */
  async function mount(frame, { scheme, five = 64, week = 38.2 } = {}) {
    const [html, css] = await Promise.all(["../../ui/index.html", "../../ui/style.css"].map((u) => fetch(u).then((r) => r.text())));
    let style = css;
    if (scheme) {
      style = style.split(DARK_QUERY).join(scheme === "dark" ? "@media all" : "@media not all") + `:root { color-scheme: ${scheme}; }`;
      frame.style.colorScheme = scheme;
    }
    frame.limitlaneTauri = stubTauri(five, week);
    const head = '<base href="../../ui/"><script>window.__TAURI__ = window.frameElement.limitlaneTauri;<\/script>';
    const page = html
      .replace("<head>", () => "<head>" + head)
      .replace('<link rel="stylesheet" href="style.css">', () => `<style>${style}</style>`);
    await new Promise((resolve) => {
      frame.addEventListener("load", () => setTimeout(resolve, 80), { once: true });
      frame.srcdoc = page;
    });
    return frame.contentDocument;
  }

  window.LimitlaneDemo = { mount };
})();
