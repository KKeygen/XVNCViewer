declare module "@novnc/novnc" {
  export default class RFB extends EventTarget {
    constructor(
      target: HTMLElement,
      url: string,
      options?: {
        credentials?: Record<string, string>;
        shared?: boolean;
        repeaterID?: string;
        wsProtocols?: string | string[];
      },
    );

    viewOnly: boolean;
    showDotCursor: boolean;
    focusOnClick: boolean;
    scaleViewport: boolean;
    resizeSession: boolean;
    clipViewport: boolean;
    background: string;
    qualityLevel: number;
    compressionLevel: number;

    connect(): void;
    disconnect(): void;
    sendCredentials(credentials: Record<string, string>): void;
    clipboardPasteFrom(text: string): void;
    sendCtrlAltDel(): void;
    toDataURL(type?: string, quality?: number): string;
    focus(): void;
    blur(): void;
  }
}
