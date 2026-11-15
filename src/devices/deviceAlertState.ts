/** Grace period past the 120ms exit animation before forcing `deviceAlertOnScreen` to reset. */
export const DEVICE_ALERT_RECOVERY_MS = 250;

/**
 * Decides whether an incoming alert replaces the current one immediately or waits in the queue.
 * Different device IDs are queued (deduplicated and capped) while the current alert plays.
 */
export function enqueueDeviceAlert<T extends { id: string }>(
	current: T | null,
	queue: readonly T[],
	incoming: T,
	maxQueue = 3,
): { action: "present" | "queue"; queue: T[] } {
	if (current && current.id !== incoming.id) {
		const filtered = queue.filter((queued) => queued.id !== incoming.id);
		return {
			action: "queue",
			queue: [...filtered, incoming].slice(-maxQueue),
		};
	}
	return {
		action: "present",
		queue: [...queue],
	};
}

/** Whether hovering the notch should leave the active `fernMode` intact. */
export function shouldPreserveModeOnHover(fernMode: string): boolean {
	return fernMode === "shelf" || fernMode === "device";
}

/** Whether a non-device `fernMode` left a stale `deviceAlert` behind that must be cleared. */
export function shouldClearDeviceAlertOnModeChange(fernMode: string, hasAlert: boolean): boolean {
	return hasAlert && fernMode !== "device";
}

/** Whether a recovery timer should clear `deviceAlertOnScreen` if `onExitComplete` does not fire. */
export function shouldScheduleOnScreenRecovery(onScreen: boolean, fernMode: string, hasAlert: boolean): boolean {
	return onScreen && !(fernMode === "device" && hasAlert);
}
