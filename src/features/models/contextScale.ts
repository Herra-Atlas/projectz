/**
 * A logarithmic scale for a slider whose useful range spans orders of magnitude.
 *
 * ## Why not a plain linear track
 *
 * Context runs from a few hundred tokens to well over a hundred thousand. A
 * linear track over that range spends almost all of its pixels on the top end:
 * 128K is roughly 250× 512, so a fixed-width slider makes each pixel worth
 * hundreds of tokens at the bottom and thousands near the top. The settings
 * people actually change -- 4K, 8K, 32K -- land within a few pixels of each
 * other, which makes the control useless for exactly the values worth choosing.
 *
 * So the thumb travels evenly through the *ratios* rather than through the
 * numbers. Doubling the context is the same distance on screen at any point,
 * which is also how the number is usually reasoned about: "double it", "half
 * it", "the same again but bigger".
 *
 * ## Why the value is rounded to significant digits, not to a fixed step
 *
 * The first version quantised to the slider's `step` of 512. That is wrong for a
 * logarithmic track: near the bottom, one thumb step is worth about three tokens,
 * so nearly every position floored to the same 512. Two thirds of the track was
 * dead -- dragging through it moved the thumb while the number sat still -- and
 * the number jumped 9,216 to 31,744 to 131,072 between quarters.
 *
 * A fixed step and a log scale fight each other, because the step is constant
 * while the value per step is not. Rounding to significant digits instead gives
 * a step that scales with the value: about 1 near 512, about 5 near 5,000, about
 * 50 near 50,000. Every position moves the number, and the numbers stay readable
 * rather than becoming 13,847.
 *
 * ## Both ends are exact
 *
 * The exponent maps 0 to `min` and 1 to `max`, so the true limits are reachable
 * and neither is a rounding artefact -- which matters here, because `max` is the
 * model's declared window and is often an awkward number like 131,072 that
 * significant-digit rounding would otherwise round down to 130,000.
 */

/** How many discrete positions the thumb has. */
export const SCALE_STEPS = 2000;

/**
 * Significant digits kept in a slider value.
 *
 * Three rather than two: two collapses everything above 1,000 to multiples of
 * 100, which put 8K and 32K -- the two settings people change most -- eight
 * positions apart on the low half of the track.
 */
const DIGITS = 3;

/**
 * Rounds to a fixed number of significant digits.
 *
 * The magnitude is found by logarithm rather than by looping over powers of ten,
 * which matters because this runs on every step of a drag.
 */
const roundToDigits = (value: number, digits: number) => {
  if (value <= 0) return 0;
  const magnitude = 10 ** Math.floor(Math.log10(value));
  const quantum = magnitude / 10 ** (digits - 1);
  return Math.round(value / quantum) * quantum;
};

/**
 * The value at a thumb position.
 *
 * `position` is in thumb steps, `[0, SCALE_STEPS]`. The result is always within
 * `[min, max]` and never rounds *up* past `max`, so dragging to the end of the
 * track cannot hand the model a context larger than the one it was pointed at.
 */
export const scaleToValue = (position: number, min: number, max: number) => {
  if (max <= min) return min;
  const clamped = Math.min(Math.max(position, 0), SCALE_STEPS);
  // The ends are returned exactly rather than rounded. `max` is the model's
  // declared window, and rounding it is the one value that must not move.
  if (clamped === 0) return min;
  if (clamped === SCALE_STEPS) return max;
  const raw = min * (max / min) ** (clamped / SCALE_STEPS);
  return Math.min(max, Math.max(min, roundToDigits(raw, DIGITS)));
};

/**
 * The position that produces `value`.
 *
 * The inverse of {@link scaleToValue}, which matters because a controlled slider
 * has to be told where the thumb is when the value changed by some other route
 * -- the number input, or a setting loaded from disk. Without an exact inverse
 * the thumb lags the number by a step, and rounding means it will not always be
 * perfectly reversible either, so the nearest position is chosen rather than an
 * exact one.
 */
export const valueToScale = (value: number, min: number, max: number) => {
  if (max <= min) return 0;
  const clamped = Math.min(Math.max(value, min), max);
  return Math.round((Math.log(clamped / min) / Math.log(max / min)) * SCALE_STEPS);
};