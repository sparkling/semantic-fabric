import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import {
  createPostgresObservationQualificationReceipt,
  type PostgresObservationQualificationInput,
  type PostgresObservationQualificationReceipt,
} from './postgres-observation-qualification.js';

const execFileAsync = promisify(execFile);

export async function runPostgresObservationQualificationProbe(
  image: string,
  probe: string,
): Promise<PostgresObservationQualificationReceipt> {
  if (!/^postgres:\S+$/.test(image)) throw new Error('qualification image must be a pinned postgres tag');
  const { stdout } = await execFileAsync('docker', [
    'run', '--rm', '--network', 'none', image, 'sh', '-c', probe,
  ], { maxBuffer: 1024 * 1024 });
  const parsed = JSON.parse(stdout) as PostgresObservationQualificationInput;
  return createPostgresObservationQualificationReceipt(parsed);
}
