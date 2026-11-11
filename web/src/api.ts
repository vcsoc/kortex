export interface Span {
  start_line: number;
  start_col: number;
  end_line: number;
  end_col: number;
}
export interface Symbol {
  symbol_id: string;
  symbol_revision_id: string;
  name: string;
  qualified_name: string;
  file_path: string;
  kind: string;
  language: string;
  span: Span;
  signature: string;
  doc_comment?: string;
  content_hash: string;
}
export interface Edge {
  id: string;
  source_id: string;
  target_id: string | null;
  kind: string;
  resolution: string;
  confidence: number;
  evidence: string;
  source_span: Span;
  candidates: string[];
}
export interface Envelope<T> {
  schema_version: string;
  request_id: string;
  repo_id?: string;
  generation_id?: string;
  index_status?: string;
  partial: boolean;
  truncated: boolean;
  warnings?: string[];
  data: T;
  page?: { next_cursor: string | null };
}
export interface Repo {
  repo_id: string;
  generation_id: string;
}
export interface IndexedFile {
  path: string;
  language?: string;
  size_bytes?: number;
  content_hash?: string;
}
export interface Graph {
  nodes: Symbol[];
  edges: Edge[];
}
export interface Context {
  symbol: Symbol;
  owner: Symbol | null;
  callers: Edge[];
  callees: Edge[];
  references: Edge[];
  types: Edge[];
  tests: Symbol[];
}
export interface Hit {
  symbol: Symbol;
  score: number;
  why_matched: string[];
}
export class ApiError extends Error {
  constructor(
    readonly code: string,
    message: string,
    readonly requestId?: string,
  ) {
    super(message);
  }
}
/** Tokens live only in this object, never storage, URLs, telemetry or log output. */
export class Api {
  private controllers = new Set<AbortController>();
  constructor(
    private origin: string,
    private token: string,
  ) {
    const url = new URL(origin);
    if (
      url.protocol !== "http:" ||
      !["127.0.0.1", "localhost", "[::1]"].includes(url.hostname) ||
      url.username ||
      url.password ||
      url.pathname !== "/" ||
      url.search ||
      url.hash
    )
      throw new ApiError(
        "LOCAL_ONLY",
        "Use a loopback HTTP origin without a path or credentials.",
      );
    this.origin = url.origin;
    if (token.length < 32 || token.length > 4096 || !/^[!-~]+$/.test(token))
      throw new ApiError(
        "AUTH_REQUIRED",
        "Enter the explicitly configured bearer token (32..4096 printable ASCII bytes).",
      );
  }
  abort() {
    for (const c of this.controllers) c.abort();
    this.controllers.clear();
  }
  async request<T>(
    path: string,
    args?: Record<string, unknown>,
  ): Promise<Envelope<T>> {
    const controller = new AbortController();
    this.controllers.add(controller);
    const timer = setTimeout(() => controller.abort(), 10_000);
    try {
      const response = await fetch(`${this.origin}/api/v1${path}`, {
        method: args ? "POST" : "GET",
        headers: {
          Authorization: `Bearer ${this.token}`,
          ...(args ? { "Content-Type": "application/json" } : {}),
        },
        ...(args ? { body: JSON.stringify(args) } : {}),
        signal: controller.signal,
        credentials: "omit",
        cache: "no-store",
        redirect: "error",
        referrerPolicy: "no-referrer",
      });
      const reader = response.body?.getReader();
      if (!reader)
        throw new ApiError("INVALID_RESPONSE", "Response body is missing.");
      const chunks: Uint8Array[] = [];
      let length = 0;
      while (true) {
        const chunk = await reader.read();
        if (chunk.done) break;
        length += chunk.value.length;
        if (length > 1_048_576) {
          await reader.cancel();
          throw new ApiError(
            "RESPONSE_BUDGET",
            "Browser response exceeded 1 MiB.",
          );
        }
        chunks.push(chunk.value);
      }
      const bytes = new Uint8Array(length);
      let offset = 0;
      for (const chunk of chunks) {
        bytes.set(chunk, offset);
        offset += chunk.length;
      }
      const value = JSON.parse(new TextDecoder().decode(bytes));
      if (!response.ok || value.error)
        throw new ApiError(
          value.error?.code ?? `HTTP_${response.status}`,
          value.error?.message ?? "API request failed.",
          value.request_id ?? response.headers.get("x-request-id") ?? undefined,
        );
      if (value.schema_version !== "1.0" || !value.data)
        throw new ApiError("INVALID_RESPONSE", "Unsupported response schema.");
      return value as Envelope<T>;
    } catch (error) {
      if (error instanceof DOMException && error.name === "AbortError")
        throw new ApiError(
          "CANCELLED",
          "Request cancelled or exceeded the 10-second browser deadline.",
        );
      throw error;
    } finally {
      clearTimeout(timer);
      this.controllers.delete(controller);
    }
  }
}
export function download(blob: Blob, name: string) {
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}
export function exportJSON(value: unknown, name: string) {
  download(
    new Blob([JSON.stringify(value, null, 2)], { type: "application/json" }),
    name,
  );
}
