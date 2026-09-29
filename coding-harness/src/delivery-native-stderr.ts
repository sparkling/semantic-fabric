// SPDX-License-Identifier: MIT
import { StringDecoder } from 'node:string_decoder';
import { redactDiagnosticText, stripControls } from './delivery-diagnostics.js';

/** Redact complete lines before tail truncation; oversize lines are withheld, never sliced raw. */
export class NativeStderrTail {
  private decoder = new StringDecoder('utf8');
  private pending = '';
  private bytes = 0;
  private dropping = false;
  private privateKey = false;
  private tail = Buffer.alloc(0);
  private readonly environment: NodeJS.ProcessEnv;
  constructor(environment: NodeJS.ProcessEnv, private readonly limit = 8192) {
    this.environment = { ...environment };
    let index = 0;
    for (const [key, value] of Object.entries(environment)) {
      if (value && /TOKEN|SECRET|PASSWORD|API_KEY|CREDENTIAL/i.test(key)) {
        for (const line of value.split(/\r?\n/).filter(Boolean)) this.environment[`SECRET_LINE_${index++}`] = line;
      }
    }
  }
  push(data: Buffer): void {
    const text = this.decoder.write(data);
    let start = 0;
    for (;;) {
      const end = text.indexOf('\n', start), part = text.slice(start, end < 0 ? undefined : end);
      if (!this.dropping) {
        this.bytes += Buffer.byteLength(part);
        if (this.bytes > 65536) {
          // Discarded text may hide a split/prefixed key marker. Never release later body lines by size.
          this.privateKey = true;
          this.pending = ''; this.dropping = true;
        }
        else this.pending += part;
      }
      if (end < 0) break;
      if (this.dropping) this.keep('[oversize stderr line withheld]\n');
      else this.line(this.pending);
      this.pending = ''; this.bytes = 0; this.dropping = false; start = end + 1;
    }
  }
  finish(): string {
    const last = this.decoder.end();
    if (this.dropping) this.keep('[oversize stderr line withheld]\n');
    else if (this.pending || last) this.line(this.pending + last);
    if (this.privateKey) this.keep('[stderr diagnostics suppressed: private key end not verified]\n');
    this.pending = '';
    return this.tail.toString('utf8');
  }
  private line(raw: string): void {
    const line = stripControls(raw);
    const begins = [...line.matchAll(/-----BEGIN [^-\r\n]*PRIVATE KEY(?: BLOCK)?-----/g)];
    const ends = [...line.matchAll(/-----END [^-\r\n]*PRIVATE KEY(?: BLOCK)?-----/g)];
    if (begins.length) { this.privateKey = true; this.keep('[redacted private key]\n'); }
    if (this.privateKey) {
      if (ends.length && ends.at(-1)!.index! > (begins.at(-1)?.index ?? -1)) this.privateKey = false;
      return;
    }
    this.keep(redactDiagnosticText(line, this.environment) + '\n');
  }
  private keep(text: string): void {
    this.tail = Buffer.concat([this.tail, Buffer.from(text)]).subarray(-this.limit);
  }
}
