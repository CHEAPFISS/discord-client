// Хелперы DOM.

/** document.getElementById с гарантией наличия (id статичны в index.html). */
export function el<T extends HTMLElement = HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) throw new Error(`Элемент #${id} не найден в index.html`);
  return node as T;
}
