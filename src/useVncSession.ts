import { useCallback, useEffect, useRef, useState } from "react";
import RFB from "@novnc/novnc";
import type { ConnectionState } from "./types";

interface ConnectOptions {
  wsUrl: string;
  username?: string;
  password?: string;
  viewOnly?: boolean;
}

export function useVncSession() {
  const targetRef = useRef<HTMLDivElement | null>(null);
  const rfbRef = useRef<RFB | null>(null);
  const [state, setState] = useState<ConnectionState>("idle");
  const [desktopName, setDesktopName] = useState("No session");
  const [lastError, setLastError] = useState<string | null>(null);

  const captureThumbnail = useCallback(() => {
    try {
      return rfbRef.current?.toDataURL("image/webp", 0.58);
    } catch {
      return undefined;
    }
  }, []);

  const releaseInput = useCallback(() => {
    rfbRef.current?.blur?.();
    targetRef.current?.blur();
  }, []);

  const disconnect = useCallback(() => {
    rfbRef.current?.disconnect();
    rfbRef.current = null;
    setState("disconnected");
  }, []);

  const setSessionViewOnly = useCallback((viewOnly: boolean) => {
    if (rfbRef.current) {
      rfbRef.current.viewOnly = viewOnly;
    }
  }, []);

  const connect = useCallback(
    ({ wsUrl, username, password, viewOnly }: ConnectOptions) => {
      if (!targetRef.current) return;

      rfbRef.current?.disconnect();
      targetRef.current.innerHTML = "";
      setState("connecting");
      setLastError(null);
      setDesktopName("Connecting");

      const rfb = new RFB(targetRef.current, wsUrl, {
        credentials: {
          username: username ?? "",
          password: password ?? "",
        },
      });

      rfb.scaleViewport = true;
      rfb.resizeSession = true;
      rfb.viewOnly = viewOnly ?? false;
      rfb.showDotCursor = true;
      rfb.focusOnClick = true;
      rfb.clipViewport = false;
      rfb.background = "#05070a";

      rfb.addEventListener("connect", () => {
        setState("connected");
        setDesktopName((current) => (current === "Connecting" ? "Connected" : current));
        rfb.focus();
      });

      rfb.addEventListener("disconnect", (event) => {
        const detail = (event as CustomEvent).detail;
        setState(detail?.clean ? "disconnected" : "failed");
        if (!detail?.clean) setLastError("VNC session disconnected unexpectedly.");
      });

      rfb.addEventListener("credentialsrequired", () => {
        setState("credentials");
        if (password) {
          rfb.sendCredentials({ username: username ?? "", password });
        }
      });

      rfb.addEventListener("securityfailure", (event) => {
        const detail = (event as CustomEvent).detail;
        setState("failed");
        setLastError(detail?.reason ?? "Security negotiation failed.");
      });

      rfb.addEventListener("desktopname", (event) => {
        const detail = (event as CustomEvent).detail;
        setDesktopName(detail?.name ?? "Remote desktop");
      });

      rfbRef.current = rfb;
    },
    [],
  );

  useEffect(() => () => rfbRef.current?.disconnect(), []);

  return {
    targetRef,
    state,
    desktopName,
    lastError,
    connect,
    disconnect,
    releaseInput,
    setViewOnly: setSessionViewOnly,
    captureThumbnail,
    sendCtrlAltDel: () => rfbRef.current?.sendCtrlAltDel(),
    fail: (message: string) => {
      setState("failed");
      setLastError(message);
      setDesktopName("Connection failed");
    },
  };
}
