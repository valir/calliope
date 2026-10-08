import { describe, it, expect } from 'vitest';
import { formatGain, sliderToGain, gainToSlider } from './gain-format';

describe('gain-format', () => {
  describe('formatGain', () => {
    it('formats various dB values', () => {
      expect(formatGain(null)).toBe('Off');
      expect(formatGain(-60)).toBe('Off');
      expect(formatGain(0)).toBe('0.0 dB');
      expect(formatGain(6.5)).toBe('+6.5 dB');
      expect(formatGain(-12)).toBe('-12.0 dB');
      expect(formatGain(12)).toBe('+12.0 dB');
      expect(formatGain(-0.5)).toBe('-0.5 dB');
    });
  });

  describe('sliderToGain', () => {
    it('converts slider value to gain', () => {
      expect(sliderToGain(-60)).toBeNull();
      expect(sliderToGain(-59.5)).toBe(-59.5);
    });
  });

  describe('gainToSlider', () => {
    it('converts gain to slider value', () => {
      expect(gainToSlider(null)).toBe(-60);
      expect(gainToSlider(20)).toBe(12);
    });
  });
});
