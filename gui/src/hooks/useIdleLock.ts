import { useEffect } from "react";

const ACTIVITY_EVENTS = ["mousemove", "mousedown", "keydown", "touchstart", "wheel"] as const;

export function useIdleLock(seconds: number, onLock: () => void) {
  useEffect(() => {
    if (seconds <= 0) return;
    let timer = window.setTimeout(onLock, seconds * 1000);
    const reset = () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(onLock, seconds * 1000);
    };
    ACTIVITY_EVENTS.forEach((ev) => window.addEventListener(ev, reset, { passive: true }));
    return () => {
      window.clearTimeout(timer);
      ACTIVITY_EVENTS.forEach((ev) => window.removeEventListener(ev, reset));
    };
  }, [seconds, onLock]);
}
