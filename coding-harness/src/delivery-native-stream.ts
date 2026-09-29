// SPDX-License-Identifier: MIT
import { StringDecoder } from 'node:string_decoder';
import { createHash } from 'node:crypto';
import type { NativeHost } from './models/types.js';

type RecordValue = Record<string, unknown>;
const object = (value: unknown): RecordValue => value !== null && typeof value === 'object' && !Array.isArray(value) ? value as RecordValue : {};
export interface NativeStreamSnapshot {
  textBytes: number; reasoningBytes: number; structuredBytes: number; toolBytes: number; toolStarts: number; toolResults: number;
  estimatedReasoningTokens: number;
  stdoutBytes: number; stderrBytes: number; malformedEvents: number;
}

/** Decode bounded frames, retain only the final Claude envelope and content-free counters. */
export class NativeStreamDecoder {
  readonly counts: NativeStreamSnapshot = { textBytes: 0, reasoningBytes: 0, structuredBytes: 0, toolBytes: 0, toolStarts: 0, toolResults: 0,
    estimatedReasoningTokens: 0,
    stdoutBytes: 0, stderrBytes: 0, malformedEvents: 0 };
  private decoder = new StringDecoder('utf8');
  private pending = '';
  private pendingBytes = 0;
  private dropping = false;
  private finalValue: string | undefined;
  private failure: string | undefined;
  private structuredBlocks = new Set<number>();
  private toolBlocks = new Set<number>();
  private thinkingBlocks = new Set<number>();
  private tools = new Map<string, { started: boolean; completed: boolean; bytes: number; output: number }>();
  private stdoutHash = createHash('sha256');
  private stderrHash = createHash('sha256');
  constructor(readonly host: NativeHost, private readonly activity: () => void,
    private readonly event: (kind: 'tool-start' | 'tool-result') => void,
    private readonly maxFrameBytes = 8_000_000, private readonly diagnostic: (message: string) => void = () => {}) {}
  stdout(data: Buffer): void {
    this.stdoutHash.update(data);
    this.counts.stdoutBytes += data.length;
    const text = this.decoder.write(data);
    let start = 0;
    for (;;) {
      const end = text.indexOf('\n', start);
      const part = text.slice(start, end < 0 ? undefined : end);
      if (!this.dropping) {
        const bytes = Buffer.byteLength(part);
        if (this.pendingBytes + bytes > this.maxFrameBytes) {
          this.pending = ''; this.pendingBytes = 0; this.dropping = true; this.failure = 'native-stream-frame-limit';
        } else { this.pending += part; this.pendingBytes += bytes; }
      }
      if (end < 0) break;
      if (!this.dropping) this.line(this.pending);
      this.pending = ''; this.pendingBytes = 0; this.dropping = false; start = end + 1;
    }
  }
  stderr(data: Buffer): void { this.counts.stderrBytes += data.length; this.stderrHash.update(data); }
  finish(): void {
    const tail = this.decoder.end();
    if (!this.dropping) this.line(this.pending + tail);
    this.pending = '';
    if (this.host === 'claude-code' && this.finalValue === undefined) this.failure ??= 'native-stream-no-result';
  }
  digests() { return { stdoutSha256: this.stdoutHash.copy().digest('hex'), stderrSha256: this.stderrHash.copy().digest('hex') }; }
  output(): string { return this.host === 'claude-code' ? this.finalValue ?? '' : JSON.stringify({ type: 'native-stream-summary', ...this.counts, ...this.digests() }); }
  error(): string | undefined { return this.failure; }
  private content(kind: 'textBytes' | 'reasoningBytes' | 'structuredBytes' | 'toolBytes', value: unknown): void {
    if (typeof value !== 'string' || value.length === 0) return;
    this.counts[kind] += Buffer.byteLength(value); this.activity();
  }
  private item(id: unknown) {
    if (typeof id !== 'string' || !id || id.length > 256) return undefined;
    let item = this.tools.get(id);
    if (!item) {
      if (this.tools.size >= 8192) { this.failure = 'native-stream-item-limit'; return undefined; }
      item = { started: false, completed: false, bytes: 0, output: 0 }; this.tools.set(id, item);
    }
    return item;
  }
  private tool(id: unknown, completed: boolean): void {
    const item = this.item(id); if (!item) return;
    const key = completed ? 'completed' : 'started';
    if (item[key] || (!completed && item.completed)) return;
    item[key] = true; this.activity();
    if (completed) { this.counts.toolResults++; this.event('tool-result'); }
    else { this.counts.toolStarts++; this.event('tool-start'); }
  }
  private line(line: string): void {
    if (!line.trim()) return;
    let parsed: unknown;
    try { parsed = JSON.parse(line); } catch { this.counts.malformedEvents++; return; }
    const row = object(parsed);
    if (this.host === 'claude-code') {
      if (row.type === 'result') {
        if (this.finalValue !== undefined) this.failure = 'native-stream-multiple-results';
        else this.finalValue = line;
        return;
      }
      if (row.type === 'stream_event') {
        const event = object(row.event), delta = object(event.delta), block = object(event.content_block);
        if (event.type === 'message_start' || event.type === 'message_stop') { this.structuredBlocks.clear(); this.toolBlocks.clear(); this.thinkingBlocks.clear(); }
        if (event.type === 'content_block_start' && block.type === 'thinking'
          && Number.isSafeInteger(event.index) && (event.index as number) >= 0 && this.thinkingBlocks.size < 1024) {
          this.thinkingBlocks.add(event.index as number);
        }
        if (event.type === 'content_block_delta') {
          if (delta.type === 'text_delta') this.content('textBytes', delta.text);
          if (delta.type === 'thinking_delta') {
            this.content('reasoningBytes', delta.thinking);
            // Native redacted reasoning carries a delta estimate, not readable thinking bytes.
            const estimate = delta.estimated_tokens;
            if (this.thinkingBlocks.has(event.index as number) && typeof estimate === 'number'
              && Number.isSafeInteger(estimate) && estimate > 0
              && Number.isSafeInteger(this.counts.estimatedReasoningTokens + estimate)) {
              this.counts.estimatedReasoningTokens += estimate; this.activity();
            }
          }
          if (delta.type === 'input_json_delta' && this.structuredBlocks.has(event.index as number)) this.content('structuredBytes', delta.partial_json);
          else if (delta.type === 'input_json_delta' && this.toolBlocks.has(event.index as number)) this.content('toolBytes', delta.partial_json);
        }
        if (event.type === 'content_block_start' && block.type === 'tool_use') {
          if (typeof block.id === 'string' && Number.isSafeInteger(event.index) && this.toolBlocks.size < 1024) {
            this.toolBlocks.add(event.index as number);
            if (block.name === 'StructuredOutput') this.structuredBlocks.add(event.index as number);
            this.tool(block.id, false);
          }
        }
        if (event.type === 'content_block_stop') { this.structuredBlocks.delete(event.index as number); this.toolBlocks.delete(event.index as number); this.thinkingBlocks.delete(event.index as number); }
      }
      if (row.type === 'user') {
        const content = object(row.message).content;
        if (Array.isArray(content)) for (const block of content) if (object(block).type === 'tool_result') {
          this.tool(object(block).tool_use_id, true);
        }
      }
    } else {
      // Codex 0.157.1 exec_events.rs emits item snapshots, not Responses API deltas.
      const item = object(row.item), state = ['item.started','item.updated','item.completed'].includes(String(row.type)) ? this.item(item.id) : undefined;
      if (state && ['agent_message','reasoning'].includes(String(item.type)) && typeof item.text === 'string') {
        const size = Buffer.byteLength(item.text), growth = Math.max(0, size - state.bytes); state.bytes = Math.max(state.bytes, size);
        if (growth) { this.counts[item.type === 'reasoning' ? 'reasoningBytes' : 'textBytes'] += growth; this.activity(); }
      }
      if (state && ['command_execution','mcp_tool_call','collab_tool_call','web_search','file_change'].includes(String(item.type))) {
        if (row.type === 'item.started') this.tool(item.id, false);
        if (row.type === 'item.completed') this.tool(item.id, true);
        if (typeof item.aggregated_output === 'string') {
          const size = Buffer.byteLength(item.aggregated_output), growth = Math.max(0, size - state.output); state.output = Math.max(state.output, size);
          if (growth) { this.counts.toolBytes += growth; this.activity(); }
        }
      }
      if (row.type === 'error' || row.type === 'turn.failed') {
        const message = row.type === 'error' ? row.message : object(row.error).message;
        if (typeof message === 'string') this.diagnostic(message);
        // Codex error notices can describe recoverable reconnects; only turn.failed is terminal.
        if (row.type === 'turn.failed') this.failure = 'native-stream-error';
      }
    }
  }
}
