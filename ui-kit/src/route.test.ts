import { describe, expect, it } from 'vitest';
import { hasPairing, MAX_VALUE, readRoute, routeHash, routeHref } from './route';

describe('readRoute', () => {
  it('reads the parameters of the fragment, decoded, with or without the #', () => {
    expect(readRoute('#view=board&task=area%2Fname')).toEqual({ view: 'board', task: 'area/name' });
    expect(readRoute('view=list')).toEqual({ view: 'list' });
    expect(readRoute('')).toEqual({});
  });

  it('never reads the pairing code, and leaves out what is not a part of a route', () => {
    const code = 'ab'.repeat(16);
    expect(readRoute(`#pair=${code}&view=plan`)).toEqual({ view: 'plan' });
    expect(readRoute('#view&=x&View=a&1x=b&v=%E0%A4%A&w=')).toEqual({});
    expect(readRoute('#task=%0Aevil')).toEqual({ task: 'evil' });
    expect(readRoute('#task=a%00b')).toEqual({});
    expect(readRoute(`#task=${'x'.repeat(MAX_VALUE + 1)}`)).toEqual({});
  });

  it('takes the first part of a key', () => {
    expect(readRoute('#view=a&view=b')).toEqual({ view: 'a' });
  });

  it('does not let a key of the prototype in', () => {
    const route = readRoute('#constructor=x&__proto__=y');
    expect(Object.keys(route)).toEqual(['constructor']);
    expect(Object.getPrototypeOf(route)).toBe(Object.prototype);
  });
});

describe('routeHash and routeHref', () => {
  it('writes the parameters in their order, encoded; nothing for an empty route', () => {
    expect(routeHash({ view: 'board', task: 'a/b c&d' })).toBe('#view=board&task=a%2Fb%20c%26d');
    expect(routeHash({})).toBe('');
    expect(routeHref({ pathname: '/', search: '?x=1' }, { view: 'plan' })).toBe('/?x=1#view=plan');
  });

  it('reads back what it wrote', () => {
    const route = { view: 'list', task: 'tasks/ä ö=&#?' };
    expect(readRoute(routeHash(route))).toEqual(route);
  });

  it("refuses a key a route cannot hold: that is the app's mistake", () => {
    expect(() => routeHash({ pair: 'x' })).toThrow();
    expect(() => routeHash({ View: 'x' })).toThrow();
  });

  it('leaves out a value from the data it cannot carry or read back the same, and does not throw', () => {
    const long = 'x'.repeat(MAX_VALUE + 1);
    for (const task of ['', 'a\nb', long, ' padded', 'padded ']) {
      expect(routeHash({ view: 'list', task })).toBe('#view=list');
    }
    expect(routeHash({ task: long })).toBe('');
  });
});

describe('hasPairing', () => {
  it('sees a pairing part, valid or not, and nothing else', () => {
    expect(hasPairing('#pair=zz')).toBe(true);
    expect(hasPairing('#view=a&pair=1')).toBe(true);
    expect(hasPairing('#view=pair')).toBe(false);
    expect(hasPairing('')).toBe(false);
  });
});
