// The screen's own closed sets (VFS-1 R8, AC-11): the values of the date and size selects.
// Their meaning is in main.js, their Spanish labels in strings.js.

export const DateChoice = Object.freeze({
  ANY: 'any',
  WEEK: 'week',
  MONTH: 'month',
  YEAR: 'year',
  CUSTOM: 'custom',
});

export const SizeChoice = Object.freeze({
  ANY: 'any',
  SMALL: 'small',
  MEDIUM: 'medium',
  LARGE: 'large',
  HUGE: 'huge',
});
