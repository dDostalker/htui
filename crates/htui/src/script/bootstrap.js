globalThis.__htuiListeners = {};
globalThis.__htuiElements = {};
globalThis.__htuiTimers = {};
globalThis.__htuiFetchResolvers = {};
globalThis.__htuiAddListener = function (nodeId, type, fn) {
  var entry = globalThis.__htuiListeners[nodeId] || (globalThis.__htuiListeners[nodeId] = {});
  var arr = entry[type] || (entry[type] = []);
  arr.push(fn);
};
globalThis.__htuiGetListeners = function (nodeId, type) {
  var entry = globalThis.__htuiListeners[nodeId];
  return entry ? (entry[type] || []) : [];
};
// Single-slot registry for `on*` attribute handlers. Unlike the multicast
// listener list above, each (node, type) holds at most one function — a
// browser event-handler slot: installing overwrites, clearing removes.
globalThis.__htuiAttrHandlers = {};
globalThis.__htuiSetAttrHandler = function (nodeId, type, fn) {
  var entry = globalThis.__htuiAttrHandlers[nodeId] || (globalThis.__htuiAttrHandlers[nodeId] = {});
  entry[type] = fn;
};
globalThis.__htuiClearAttrHandler = function (nodeId, type) {
  var entry = globalThis.__htuiAttrHandlers[nodeId];
  if (entry) { delete entry[type]; }
};
globalThis.__htuiClearAllAttrHandlers = function (nodeId) {
  delete globalThis.__htuiAttrHandlers[nodeId];
};
// Live NodeList factory: `getLen()` / `getItem(i)` are native closures over
// the engine arena, so every access (length / index / enumeration) reflects
// the current DOM — a held `childNodes` reference stays live.
globalThis.__htuiLiveList = function (getLen, getItem) {
  function index(p) {
    var n = Number(p);
    return String(n) === p ? n : NaN;
  }
  var target = {
    item: function (i) {
      i = Number(i) | 0;
      return (i >= 0 && i < getLen()) ? getItem(i) : null;
    },
    forEach: function (cb, self) {
      for (var i = 0; i < getLen(); i++) cb.call(self, getItem(i), i, this);
    }
  };
  return new Proxy(target, {
    get: function (t, p) {
      if (p === 'length') return getLen();
      var i = index(p);
      if (!Number.isNaN(i) && i >= 0 && i < getLen()) return getItem(i);
      if (p === Symbol.iterator) return function () {
        var i = 0;
        return { next: function () {
          return i < getLen()
            ? { value: getItem(i++), done: false }
            : { value: undefined, done: true };
        } };
      };
      return Reflect.get(t, p);
    },
    has: function (t, p) {
      if (p === 'length') return true;
      var i = index(p);
      if (!Number.isNaN(i) && i >= 0 && i < getLen()) return true;
      return Reflect.has(t, p);
    },
    ownKeys: function () {
      var keys = [];
      for (var i = 0; i < getLen(); i++) keys.push(String(i));
      keys.push('length');
      return keys;
    },
    getOwnPropertyDescriptor: function (t, p) {
      if (p === 'length') return { get: getLen, enumerable: false, configurable: true };
      var i = index(p);
      if (!Number.isNaN(i) && i >= 0 && i < getLen()) {
        return { value: getItem(i), writable: true, enumerable: true, configurable: true };
      }
      return Reflect.getOwnPropertyDescriptor(t, p);
    },
    set: function () { return false; },
    deleteProperty: function () { return false; }
  });
};
// Live DOMStringMap factory for `el.dataset`. `get` / `set` / `remove` /
// `keys` are native closures over the engine arena (the element's `data-*`
// attributes), so every access reflects the current DOM — a held `dataset`
// reference stays live. Property names use the browser camelCase↔kebab-case
// mapping (`dataset.fooBar` <-> `data-foo-bar`); assigning `''` removes the
// attribute, and `Object.keys(dataset)` enumerates the kebab-cased names.
globalThis.__htuiDataset = function (get, set, remove, keys) {
  function kebab(p) {
    return String(p).replace(/[A-Z]/g, function (c) { return '-' + c.toLowerCase(); });
  }
  function hasData(k) {
    return keys().indexOf(k) !== -1;
  }
  var target = {
    toString: function () { return '[object DOMStringMap]'; }
  };
  return new Proxy(target, {
    get: function (t, p) {
      if (typeof p === 'symbol') return Reflect.get(t, p);
      // `toString` / `valueOf` fall back to the target so coercions such as
      // `String(dataset)` succeed; everything else is a data-* lookup.
      if (p === 'toString' || p === 'valueOf') return Reflect.get(t, p);
      if (p === 'then' || p === 'toJSON') return undefined;
      return get(kebab(p));
    },
    set: function (t, p, v) {
      if (typeof p !== 'string' || p === 'then') return false;
      var k = kebab(p);
      if (v === null || v === undefined || v === '') { remove(k); }
      else { set(k, String(v)); }
      return true;
    },
    has: function (t, p) {
      if (typeof p !== 'string') return Reflect.has(t, p);
      return hasData(kebab(p));
    },
    deleteProperty: function (t, p) {
      if (typeof p !== 'string') return false;
      remove(kebab(p));
      return true;
    },
    ownKeys: function () {
      return keys();
    },
    getOwnPropertyDescriptor: function (t, p) {
      if (typeof p !== 'string') return undefined;
      var v = get(kebab(p));
      if (v === undefined) return undefined;
      return { value: v, writable: true, enumerable: true, configurable: true };
    }
  });
};
// Live DOMTokenList factory for `el.classList`. `getAttr()` / `setAttr()`
// are native closures over the engine arena (the element's `class`
// attribute), so every access reflects the current DOM — a held `classList`
// reference stays live. Mirrors the browser semantics: token order and first
// occurrences are preserved, empty / whitespace-containing tokens are
// rejected (`SyntaxError`), and add / remove / toggle / replace only rewrite
// the attribute when the token set actually changed.
globalThis.__htuiClassList = function (getAttr, setAttr) {
  function tokens() {
    var raw = getAttr();
    return raw ? raw.split(/\s+/).filter(Boolean) : [];
  }
  function write(list) {
    setAttr(list.join(' '));
  }
  function check(token) {
    if (token === '') throw new SyntaxError('The token provided must not be empty.');
    if (/[ \t\n\f\r]/.test(token)) {
      throw new SyntaxError('The token provided contains HTML space characters.');
    }
  }
  function index(p) {
    if (typeof p !== 'string') return NaN;
    var n = Number(p);
    return String(n) === p ? n : NaN;
  }
  // Iterators returned by keys / values / entries are also iterable
  // (WebIDL iterators), so `Array.from(list.keys())` works.
  function makeIter(step) {
    var it = { next: step };
    it[Symbol.iterator] = function () { return it; };
    return it;
  }
  var target = {
    item: function (i) {
      var t = tokens();
      i = Number(i) | 0;
      return (i >= 0 && i < t.length) ? t[i] : null;
    },
    contains: function (token) {
      token = String(token);
      check(token);
      return tokens().indexOf(token) !== -1;
    },
    add: function () {
      var t = tokens();
      var changed = false;
      for (var i = 0; i < arguments.length; i++) {
        var tok = String(arguments[i]);
        check(tok);
        if (t.indexOf(tok) === -1) { t.push(tok); changed = true; }
      }
      if (changed) write(t);
    },
    remove: function () {
      var t = tokens();
      var changed = false;
      for (var i = 0; i < arguments.length; i++) {
        var tok = String(arguments[i]);
        check(tok);
        var p = t.indexOf(tok);
        if (p !== -1) { t.splice(p, 1); changed = true; }
      }
      if (changed) write(t);
    },
    toggle: function (token, force) {
      token = String(token);
      check(token);
      var t = tokens();
      var p = t.indexOf(token);
      var present = p !== -1;
      if (force === undefined) {
        if (present) t.splice(p, 1); else t.push(token);
        write(t);
        return !present;
      }
      if (force) {
        if (!present) { t.push(token); write(t); }
        return true;
      }
      if (present) { t.splice(p, 1); write(t); }
      return false;
    },
    replace: function (oldToken, newToken) {
      oldToken = String(oldToken);
      newToken = String(newToken);
      check(oldToken);
      check(newToken);
      var t = tokens();
      var p = t.indexOf(oldToken);
      if (p === -1) return false;
      t.splice(p, 1);
      if (t.indexOf(newToken) === -1) t.push(newToken);
      write(t);
      return true;
    },
    forEach: function (cb, self) {
      var t = tokens();
      for (var i = 0; i < t.length; i++) cb.call(self, t[i], i, this);
    },
    toString: function () {
      return getAttr() || '';
    },
    entries: function () {
      var i = 0;
      var t = tokens();
      return makeIter(function () {
        return i < t.length ? { value: [i, t[i++]], done: false } : { value: undefined, done: true };
      });
    },
    keys: function () {
      var i = 0;
      var t = tokens();
      return makeIter(function () {
        return i < t.length ? { value: i++, done: false } : { value: undefined, done: true };
      });
    },
    values: function () {
      var i = 0;
      var t = tokens();
      return makeIter(function () {
        return i < t.length ? { value: t[i++], done: false } : { value: undefined, done: true };
      });
    }
  };
  target[Symbol.iterator] = target.values;
  // `value` reads / writes the underlying `class` attribute (configurable so
  // the proxy's `ownKeys` can omit it without violating the invariant).
  Object.defineProperty(target, 'value', {
    get: function () { return getAttr() || ''; },
    set: function (v) { setAttr(String(v)); },
    enumerable: false,
    configurable: true
  });
  return new Proxy(target, {
    get: function (t, p) {
      if (p === 'length') return tokens().length;
      var i = index(p);
      if (!Number.isNaN(i)) {
        var tk = tokens();
        if (i >= 0 && i < tk.length) return tk[i];
      }
      return Reflect.get(t, p);
    },
    has: function (t, p) {
      if (p === 'length') return true;
      var i = index(p);
      if (!Number.isNaN(i)) {
        var tk = tokens();
        return i >= 0 && i < tk.length;
      }
      return Reflect.has(t, p);
    },
    ownKeys: function () {
      var keys = [];
      var t = tokens();
      for (var i = 0; i < t.length; i++) keys.push(String(i));
      keys.push('length');
      return keys;
    },
    getOwnPropertyDescriptor: function (t, p) {
      if (p === 'length') return { get: function () { return tokens().length; }, enumerable: false, configurable: true };
      var i = index(p);
      if (!Number.isNaN(i)) {
        var tk = tokens();
        if (i >= 0 && i < tk.length) {
          return { value: tk[i], writable: true, enumerable: true, configurable: true };
        }
      }
      return Reflect.getOwnPropertyDescriptor(t, p);
    },
    set: function (t, p, v) {
      // `value` is the only writable property (the `class` attribute);
      // everything else (indexed keys, methods, `length`) is read-only.
      if (p === 'value') {
        var d = Object.getOwnPropertyDescriptor(t, 'value');
        if (d && typeof d.set === 'function') { d.set.call(t, v); return true; }
      }
      return false;
    },
    deleteProperty: function () { return false; }
  });
};