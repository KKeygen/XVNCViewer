export type ConnectionState =
  | "idle"
  | "connecting"
  | "connected"
  | "credentials"
  | "disconnected"
  | "failed";

export interface SavedConnection {
  id: string;
  name: string;
  host: string;
  port: number;
  wsUrl: string;
  username: string;
  lastConnectedAt?: string;
  thumbnail?: string;
}

export interface ViewerScreen {
  id: string;
  name: string;
  thumbnail?: string;
  isPrimary?: boolean;
}
