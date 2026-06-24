// Typed wrappers around the Tauri command bridge (src-tauri/src/commands.rs).
import { invoke } from "@tauri-apps/api/core";

export type Reachability =
  | "reachable"
  | "dns_blocked"
  | "tcp_blocked"
  | "tls_reset"
  | "timeout";

export interface DomainCheck {
  text: Reachability;
  voice?: Reachability;
}

export interface TcpStrategy {
  desync: string;
  split_pos: string;
  ttl: number;
  fooling: string;
  repeats: number;
}

export interface UdpStrategy {
  desync: string;
  ttl: number;
  repeats: number;
}

export interface Strategy {
  tcp: TcpStrategy;
  udp_quic?: UdpStrategy;
}

export interface NetworkFingerprint {
  gateway_mac?: string;
  link_type?: string;
  subnet?: string;
  iface?: string;
}

export interface TestResults {
  text: boolean;
  voice: boolean;
  last_checked?: string;
}

export interface Profile {
  id: string;
  name: string;
  created_at: string;
  network_fingerprint: NetworkFingerprint;
  domains: string[];
  strategy: Strategy;
  test_results: TestResults;
  auto_test_interval_min: number;
}

export interface Settings {
  language: string;
  theme: string;
  scope: string;
  auto_test_interval_min: number;
  autostart: boolean;
}

export type ProbeOutcome =
  | { outcome: "already_open"; check: DomainCheck }
  | { outcome: "found"; strategy: Strategy; check: DomainCheck }
  | { outcome: "not_found" };

export const api = {
  checkDomain: (domain: string, withVoice: boolean) =>
    invoke<DomainCheck>("check_domain_cmd", { domain, withVoice }),
  networkFingerprint: () => invoke<NetworkFingerprint>("network_fingerprint"),
  solve: (domains: string[], withVoice: boolean) =>
    invoke<ProbeOutcome>("solve", { domains, withVoice, nfqws: null }),
  createProfile: (domains: string[], strategy: Strategy, check: DomainCheck) =>
    invoke<Profile>("create_profile", { domains, strategy, check }),
  listProfiles: () => invoke<Profile[]>("list_profiles"),
  defaultProfileId: () => invoke<string | null>("default_profile_id"),
  renameProfile: (id: string, name: string) =>
    invoke<void>("rename_profile", { id, name }),
  deleteProfile: (id: string) => invoke<void>("delete_profile", { id }),
  setDefaultProfile: (id: string) =>
    invoke<void>("set_default_profile", { id }),
  exportProfile: (id: string) => invoke<string>("export_profile_cmd", { id }),
  importProfile: (json: string) =>
    invoke<Profile>("import_profile_cmd", { json }),
  updateProfileStrategy: (id: string, strategy: Strategy) =>
    invoke<void>("update_profile_strategy", { id, strategy }),
  updateProfileDomains: (id: string, domains: string[]) =>
    invoke<void>("update_profile_domains", { id, domains }),
  engineApply: (id: string) => invoke<void>("engine_apply", { id }),
  engineRevert: () => invoke<void>("engine_revert"),
  engineStatus: () => invoke<boolean>("engine_status"),
  setAlwaysOn: (enabled: boolean) =>
    invoke<void>("set_always_on", { enabled }),
  serviceStatus: () =>
    invoke<{ enabled: boolean; active: boolean }>("service_status"),
  getSettings: () => invoke<Settings>("get_settings"),
  setSettings: (settings: Settings) =>
    invoke<void>("set_settings", { settings }),
  discordDomains: () => invoke<string[]>("discord_domains"),
};
