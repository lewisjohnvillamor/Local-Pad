// Admin API helpers. Every state-changing call carries the custom header
// the server requires as its cross-origin guard.

export async function apiGet<T>(path: string): Promise<T> {
  const response = await fetch(path);
  if (!response.ok) throw new Error(`${path}: ${response.status}`);
  return (await response.json()) as T;
}

export async function apiPost<T = unknown>(path: string, body?: unknown): Promise<T> {
  const response = await fetch(path, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "X-LocalPad-Admin": "1",
    },
    body: JSON.stringify(body ?? {}),
  });
  if (!response.ok) throw new Error(`${path}: ${response.status}`);
  const text = await response.text();
  return (text ? JSON.parse(text) : {}) as T;
}
