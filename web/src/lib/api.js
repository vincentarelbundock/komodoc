// Talking to the server.
//
// A browser cannot set a custom header on a cross-origin request without a
// CORS preflight, which the server never grants -- so this header is proof, to
// the server, that a state-changing request came from this page and not from a
// hostile document on the sibling documents host. It goes on every write and
// on the listing, never on plain navigation.
export const SHELL_HEADERS = { "X-Komodoc-Client": "shell" };

async function json(response) {
  if (!response.ok) {
    const body = await response.json().catch(() => ({}));
    throw new Error(body.error || `${response.status}`);
  }
  return response.json();
}

export const get = (path) => fetch(path).then(json);

export const getPrivate = (path) => fetch(path, { headers: SHELL_HEADERS }).then(json);

export const post = (path, body) =>
  fetch(path, {
    method: "POST",
    headers: { "content-type": "application/json", ...SHELL_HEADERS },
    body: JSON.stringify(body ?? {}),
  }).then(json);

/// A POST whose status matters to the caller, since a refused save says what
/// to do about it and a thrown error would lose that.
export const postRaw = (path, body) =>
  fetch(path, {
    method: "POST",
    headers: { "content-type": "application/json", ...SHELL_HEADERS },
    body: JSON.stringify(body),
  });

export const upload = (form) =>
  fetch("/api/documents", { method: "POST", headers: SHELL_HEADERS, body: form });

/// The limits both this page and the server enforce, so the two never
/// disagree about what will be refused.
export const config = () => get("/api/config");

/// Who you are, which decides what every page is: what you may publish, what
/// you may comment on, and whether there is anything to sign in to.
export const me = () => get("/api/me").catch(() => ({}));

export async function signOut() {
  // A GET can be forced onto a signed-in reader cross-site, so signing out is
  // a POST carrying the same header every other state change does.
  await fetch("/auth/logout", { method: "POST", headers: SHELL_HEADERS }).catch(() => {});
  location.reload();
}

export const signInHref = () => `/auth/login?next=${encodeURIComponent(location.pathname)}`;
