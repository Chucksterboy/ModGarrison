import { useCallback, useEffect, useId, useRef, useState } from "react";
import type { GarrisonCoreHealth } from "../shared/contracts";
import {
  checkingCoreHealth,
  classifyCoreHealth,
  failedCoreHealth,
  type CoreHealthState
} from "./health";

const storageProbeKey = "__mod_garrison_health_probe__";
const defaultIntervalMs = 60_000;

export interface SystemHealthButtonProps {
  profileId?: string;
  probeCore(profileId?: string): Promise<GarrisonCoreHealth>;
  intervalMs?: number;
  ready?: boolean;
}

export function probeLocalStorage(storage: Storage = window.localStorage): boolean {
  const value = `${Date.now()}:${Math.random()}`;
  try {
    storage.setItem(storageProbeKey, value);
    const stored = storage.getItem(storageProbeKey);
    storage.removeItem(storageProbeKey);
    return stored === value;
  } catch {
    try {
      storage.removeItem(storageProbeKey);
    } catch {
      // A blocked storage provider may reject cleanup as well.
    }
    return false;
  }
}

export function SystemHealthButton({
  profileId,
  probeCore,
  intervalMs = defaultIntervalMs,
  ready = true
}: SystemHealthButtonProps) {
  const [health, setHealth] = useState<CoreHealthState>(checkingCoreHealth);
  const mountedRef = useRef(false);
  const inFlightRef = useRef(false);
  const requestSequenceRef = useRef(0);
  const detailId = useId();

  const runCheck = useCallback(async () => {
    if (!ready || inFlightRef.current) {
      return;
    }

    inFlightRef.current = true;
    const requestId = ++requestSequenceRef.current;
    if (mountedRef.current) {
      setHealth(checkingCoreHealth);
    }

    try {
      const localStorageReady = probeLocalStorage();
      const result = await probeCore(profileId || undefined);
      if (mountedRef.current && requestSequenceRef.current === requestId) {
        setHealth(classifyCoreHealth(result, localStorageReady));
      }
    } catch (caughtError) {
      if (mountedRef.current && requestSequenceRef.current === requestId) {
        setHealth(failedCoreHealth(caughtError));
      }
    } finally {
      if (requestSequenceRef.current === requestId) {
        inFlightRef.current = false;
      }
    }
  }, [probeCore, profileId, ready]);

  useEffect(() => {
    if (!ready) {
      return;
    }

    mountedRef.current = true;
    void runCheck();

    const intervalId = window.setInterval(() => {
      if (document.visibilityState === "visible") {
        void runCheck();
      }
    }, intervalMs);
    const handleVisibilityChange = () => {
      if (document.visibilityState === "visible") {
        void runCheck();
      }
    };
    document.addEventListener("visibilitychange", handleVisibilityChange);

    return () => {
      mountedRef.current = false;
      requestSequenceRef.current += 1;
      inFlightRef.current = false;
      window.clearInterval(intervalId);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
    };
  }, [intervalMs, ready, runCheck]);

  return (
    <span
      className={`status-health-control status-health--${health.phase}`}
      data-health-state={health.phase}
    >
      <button
        aria-busy={health.phase === "checking"}
        aria-describedby={detailId}
        aria-label="Check Mod Garrison system health"
        className="status-health"
        onClick={() => void runCheck()}
        type="button"
      >
        <span className="status-ecg" aria-hidden="true">
          <svg viewBox="0 0 46 28" role="presentation">
            <path className="status-ecg-baseline" d="M1 14H45" />
            <path
              className="status-ecg-trace"
              d="M1 14H10L13 11L16 22L21 4L25 18L29 12L33 14H45"
            />
          </svg>
        </span>
        <span className="status-health-copy" role="status" aria-live="polite">
          <small>Health</small>
          <strong>{health.label}</strong>
        </span>
      </button>
      <span className="status-health-detail" id={detailId}>
        {health.detail}
      </span>
    </span>
  );
}
