import { describe, expect, it } from 'vitest';
import { formatTraining, parseSpeed, parseTrainingField, readoutText } from './speed-format';

describe('speed-format', () => {
  it('parseSpeed accepts whole 25..200 with optional %', () => {
    expect(parseSpeed('85')).toBe(85);
    expect(parseSpeed(' 85 % ')).toBe(85);
    expect(parseSpeed('85%')).toBe(85);
    expect(parseSpeed('25')).toBe(25);
    expect(parseSpeed('200')).toBe(200);
    for (const t of ['20', '250', '85.5', '', 'abc', '-5']) expect(parseSpeed(t)).toBeNull();
  });
  it('readoutText', () => {
    expect(readoutText(60)).toBe('60 BPM');
    expect(readoutText(79.05)).toBe('79.1 BPM');
    expect(readoutText(0)).toBe('');
  });
  it('formatTraining', () => {
    expect(formatTraining({ loop: 0, speed_pct: 70, repeat: 2, of: 4 })).toBe('70 % · 2/4');
    expect(formatTraining({ loop: 0, speed_pct: 80, repeat: 3, of: null })).toBe('80 % · 3');
  });
  it('parseTrainingField', () => {
    expect(parseTrainingField('10', 1, 175)).toBe(10);
    expect(parseTrainingField('0', 1, 175)).toBeNull();
    expect(parseTrainingField('176', 1, 175)).toBeNull();
    expect(parseTrainingField('x', 1, 99)).toBeNull();
  });
});
