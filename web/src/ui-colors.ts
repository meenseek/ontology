/** Canvas/WebGL cannot resolve CSS variables themselves. Read the same UI
 * tokens as DOM controls; domain palettes and neutral texture masks stay separate. */
export function uiColors(element: Element = document.documentElement) {
  const style = getComputedStyle(element);
  const color = (name: string) => style.getPropertyValue(name).trim();
  return {
    primary: color("--primary"),
    strong: color("--text-strong"),
    subtle: color("--text-subtle"),
    border: color("--border"),
    warning: color("--warning"),
    historical: color("--warning-border"),
  };
}
