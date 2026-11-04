/** What Memory Saver reports through the `memory-saver-state` event. */
export interface RestState {
  resting: boolean;
  locked: boolean;
}

const RESTING_FACTOR = 4;

/** The interval for a timer with the normal pace `baseMs`; null means it should not run at all. */
export function restingInterval(baseMs: number, state: RestState): number | null {
  if (state.locked) return null;
  return state.resting ? baseMs * RESTING_FACTOR : baseMs;
}
