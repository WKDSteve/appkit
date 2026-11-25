// Signing out of the hub, the same for every app: the hub is asked to end the session (its token and its streams end
// with it), and the page forgets the session whatever the hub answers, also when it does not answer at all. Only the
// POST is the hub's; what "forget" clears (token, state, caches) is the page's, because each app holds different state.

/**
 * @param logout asks the hub to end the session (a POST); may reject or throw when the hub is down
 * @param forget clears the token and everything read with the session; always runs, once, after the hub was asked
 */
export async function signOut(logout: () => Promise<unknown>, forget: () => void): Promise<void> {
  try {
    await logout();
  } catch {
    // The hub is down or has forgotten the session already: the page forgets it either way.
  }
  forget();
}
