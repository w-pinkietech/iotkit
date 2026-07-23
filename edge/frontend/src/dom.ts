export type FormField =
  | HTMLInputElement
  | HTMLSelectElement
  | HTMLTextAreaElement;

export function query<E extends Element>(
  selector: string,
  root: ParentNode = document,
): E | null {
  return root.querySelector<E>(selector);
}

export function queryAll<E extends Element>(
  selector: string,
  root: ParentNode = document,
): E[] {
  return Array.from(root.querySelectorAll<E>(selector));
}

export function formField<T extends FormField = FormField>(
  form: HTMLFormElement,
  name: string,
): T | null {
  const candidate = form.elements.namedItem(name);
  return candidate instanceof HTMLInputElement ||
    candidate instanceof HTMLSelectElement ||
    candidate instanceof HTMLTextAreaElement
    ? (candidate as T)
    : null;
}

export function requiredFormField<T extends FormField = FormField>(
  form: HTMLFormElement,
  name: string,
): T {
  const candidate = formField<T>(form, name);
  if (!candidate) throw new Error(`missing form field: ${name}`);
  return candidate;
}

export function numericFormField(
  form: HTMLFormElement,
  name: string,
  fallback = 0,
): number {
  const value = formField(form, name)?.value.trim();
  return value ? Number(value) : fallback;
}
