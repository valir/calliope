import { describe, expect, it } from 'vitest';
import { checkUrl, INVALID_URL_MESSAGE } from './url-check';

describe('checkUrl', () => {
  it.each([
    'https://www.example.com/watch?v=abc',
    'http://host.example/a',
    '  https://host.example/x  ',
    'http://127.0.0.1:8080/x',
  ])('accepts %s', (u) => expect(checkUrl(u)).toBeNull());

  it.each([
    '',
    '   ',
    'not a url',
    'www.example.com/watch',
    'ftp://host.example/a',
    'file:///etc/passwd',
    'javascript:alert(1)',
    'https://',
    'https://user:pw@host.example/',
    'https://user@host.example/',
    'https://host.example/a b',
    'https://host.example/\u0007',
    'https://host.example/' + 'a'.repeat(2000),
  ])('rejects %j', (u) => expect(checkUrl(u)).toBe(INVALID_URL_MESSAGE));
});
