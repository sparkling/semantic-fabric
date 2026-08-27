// SPDX-License-Identifier: MIT

export interface InspectedJson {
  readonly topLevelKeyCounts: Readonly<Record<string, number>>;
  readonly topLevelNumberValues: Readonly<Record<string, readonly number[]>>;
}

const JSON_NUMBER = /-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?/y;

/**
 * Count selected root keys and inspect numeric discriminator values without
 * materializing the full document. A recognized schema can therefore be
 * delegated byte-for-byte to its versioned parser exactly once.
 */
export function inspectJsonRoot(
  serialized: string,
  keys: readonly string[],
  label: string,
): InspectedJson {
  const counts = Object.fromEntries(keys.map((key) => [key, 0]));
  const numberValues = Object.fromEntries(keys.map((key) => [key, [] as number[]]));
  try {
    scanObjectKeys(serialized, (key, depth, valueStart) => {
      if (depth !== 1 || !Object.hasOwn(counts, key)) return;
      counts[key] += 1;
      const number = readJsonNumber(serialized, valueStart);
      if (number !== undefined) numberValues[key].push(number);
    });
  } catch {
    throw new TypeError(`${label} is not valid JSON`);
  }
  return Object.freeze({
    topLevelKeyCounts: Object.freeze(counts),
    topLevelNumberValues: Object.freeze(Object.fromEntries(
      Object.entries(numberValues).map(([key, values]) => [key, Object.freeze(values)]),
    )),
  });
}

/** Validate and materialize JSON only when lexical dispatch cannot select a parser. */
export function parseJsonDocument(serialized: string, label: string): unknown {
  return parseJson(serialized, label);
}

/** Parse JSON while rejecting duplicate member names at every object depth. */
export function parseJsonWithoutDuplicateKeys(serialized: string, label: string): unknown {
  const value = parseJson(serialized, label);
  const objectKeys: Array<Set<string> | null> = [];
  scanObjectKeys(serialized, (key, depth) => {
    const seen = objectKeys[depth - 1];
    if (seen === null || seen === undefined) {
      throw new TypeError(`${label} object structure is invalid`);
    }
    if (seen.has(key)) throw new TypeError(`${label} contains duplicate JSON key: ${key}`);
    seen.add(key);
  }, {
    onOpen: (kind, depth) => { objectKeys[depth - 1] = kind === 'object' ? new Set() : null; },
    onClose: (depth) => { objectKeys.length = depth - 1; },
  });
  return value;
}

function parseJson(serialized: string, label: string): unknown {
  try {
    return JSON.parse(serialized) as unknown;
  } catch {
    throw new TypeError(`${label} is not valid JSON`);
  }
}

function scanObjectKeys(
  serialized: string,
  onKey: (key: string, depth: number, valueStart: number) => void,
  lifecycle: Readonly<{
    onOpen?: (kind: 'object' | 'array', depth: number) => void;
    onClose?: (depth: number) => void;
  }> = {},
): void {
  const containers: Array<'object' | 'array'> = [];
  let stringStart = -1;
  let escaped = false;
  for (let index = 0; index < serialized.length; index += 1) {
    const character = serialized[index];
    if (stringStart >= 0) {
      if (escaped) {
        escaped = false;
      } else if (character === '\\') {
        escaped = true;
      } else if (character === '"') {
        const next = nextNonWhitespace(serialized, index + 1);
        if (serialized[next] === ':' && containers.at(-1) === 'object') {
          const token = serialized.slice(stringStart, index + 1);
          onKey(
            JSON.parse(token) as string,
            containers.length,
            nextNonWhitespace(serialized, next + 1),
          );
        }
        stringStart = -1;
      }
      continue;
    }
    if (character === '"') {
      stringStart = index;
    } else if (character === '{' || character === '[') {
      const kind = character === '{' ? 'object' : 'array';
      containers.push(kind);
      lifecycle.onOpen?.(kind, containers.length);
    } else if (character === '}' || character === ']') {
      lifecycle.onClose?.(containers.length);
      containers.pop();
    }
  }
}

function readJsonNumber(serialized: string, start: number): number | undefined {
  JSON_NUMBER.lastIndex = start;
  const match = JSON_NUMBER.exec(serialized);
  if (match === null) return undefined;
  const end = nextNonWhitespace(serialized, start + match[0].length);
  if (serialized[end] !== ',' && serialized[end] !== '}') return undefined;
  const value = Number(match[0]);
  return Number.isFinite(value) ? value : undefined;
}

function nextNonWhitespace(serialized: string, start: number): number {
  let index = start;
  while (index < serialized.length && /\s/.test(serialized[index])) index += 1;
  return index;
}
