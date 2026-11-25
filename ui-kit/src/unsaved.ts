// A page with something typed that is not saved yet asks the browser to confirm before the tab is closed, reloaded or
// navigated away from. The browser shows its own words (a page cannot set them); the listener lives only while there is
// something to lose, so a page without changes leaves without a question.

/** Ask before the page is left; the returned function takes the question away again. */
export function warnBeforeUnload(target: Pick<Window, 'addEventListener' | 'removeEventListener'> = window): () => void {
  const onUnload = (event: Event): void => {
    event.preventDefault();
    // Older browsers read the question from the return value.
    (event as BeforeUnloadEvent).returnValue = '';
  };
  target.addEventListener('beforeunload', onUnload);
  return () => target.removeEventListener('beforeunload', onUnload);
}
