// A form that is not complete can still be sent: the send says what is missing, at the field (`aria-invalid` and
// `aria-describedby` on it, a short text beside it) and puts the focus on the first field that is wrong. A button that
// is `disabled` for an empty field leaves the person guessing why, and it takes the focus away from the person.

/** Put the focus on the first field of `root` that says it is invalid; whether there was one. */
export function focusFirstInvalid(root: ParentNode | null | undefined): boolean {
  const field = root?.querySelector<HTMLElement>('[aria-invalid="true"]');
  if (!field) return false;
  field.focus();
  return true;
}
