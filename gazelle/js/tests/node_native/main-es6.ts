import * as fs from 'fs';
import { exists } from 'node:fs';

console.log(fs.exists('foo'), exists('bar'));
