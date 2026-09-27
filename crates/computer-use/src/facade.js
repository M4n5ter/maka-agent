/*
 * Licensed to the Apache Software Foundation (ASF) under one
 * or more contributor license agreements.  See the NOTICE file
 * distributed with this work for additional information
 * regarding copyright ownership.  The ASF licenses this file
 * to you under the Apache License, Version 2.0 (the
 * "License"); you may not use this file except in compliance
 * with the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing,
 * software distributed under the License is distributed on an
 * "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
 * KIND, either express or implied.  See the License for the
 * specific language governing permissions and limitations
 * under the License.
 */

// The enclosing bootstrap supplies call/emit; neither is exposed to model code.
const ErrorClass = Error;
const stringify = JSON.stringify;
const output = (value) => {
  const error = emit(value);
  if (error) throw new ErrorClass(error.message);
};
const write = (value) =>
  output({
    kind: 'text',
    text: typeof value === 'string' ? value : (stringify(value) ?? 'undefined'),
  });
const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
const decode = (value) => {
  const padding = value.endsWith('==') ? 2 : value.endsWith('=') ? 1 : 0;
  const result = new Uint8Array(Math.floor((value.length * 3) / 4) - padding);
  let offset = 0;
  let bits = 0;
  let buffer = 0;
  for (const ch of value) {
    if (ch === '=') break;
    const n = alphabet.indexOf(ch);
    if (n < 0) throw new ErrorClass('Invalid base64 image');
    buffer = (buffer << 6) | n;
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      result[offset++] = (buffer >> bits) & 255;
    }
  }
  return result;
};
const encode = (bytes) => {
  let result = '';
  for (let i = 0; i < bytes.length; i += 3) {
    const n = (bytes[i] << 16) | ((bytes[i + 1] ?? 0) << 8) | (bytes[i + 2] ?? 0);
    result +=
      alphabet[(n >> 18) & 63] +
      alphabet[(n >> 12) & 63] +
      (i + 1 < bytes.length ? alphabet[(n >> 6) & 63] : '=') +
      (i + 2 < bytes.length ? alphabet[n & 63] : '=');
  }
  return result;
};
const emitImage = async (value) => {
  let image;
  if (typeof value === 'string') {
    const match = /^data:(image\/(?:png|jpeg|webp));base64,([A-Za-z0-9+/]+=*)$/.exec(value);
    if (!match)
      throw new ErrorClass(
        'Expected an image data URL; filesystem and URL fetching are unavailable',
      );
    image = { type: 'image', mimeType: match[1], data: match[2] };
  } else if (value?.type === 'image') {
    image = { type: 'image', mimeType: value.mimeType, data: value.data };
  } else {
    const bytes = value instanceof Uint8Array ? value : value?.bytes;
    if (!(bytes instanceof Uint8Array))
      throw new ErrorClass('Expected image bytes or an image content block');
    const mimeType =
      value.mimeType ??
      (bytes[0] === 137 ? 'image/png' : bytes[0] === 255 ? 'image/jpeg' : 'image/webp');
    image = { type: 'image', mimeType, data: encode(bytes) };
  }
  output({ kind: 'media', content: image });
};
const invoke = async (method, args = {}) => {
  const result = await call('cua', { method, ...args });
  if (!result.ok) throw new ErrorClass(result.error.message);
  return result.value;
};
let documented = false;
let reference;
const documentation = async () => {
  const text = reference ?? (reference = await invoke('documentation'));
  if (!documented) {
    write(text);
    documented = true;
  }
  return text;
};
const observation = async (method, options = {}) => {
  const value = await invoke(method, { options });
  await documentation();
  if (options.emit !== false) write(value);
  return value;
};
const bind = async (method, args) => {
  const result = await invoke(method, args);
  await documentation();
  const handle = result.handle;
  const observe = async (kind, options = {}) => {
    const value = await invoke('observe', { handle, kind, options });
    if (options.emit !== false) {
      if (value.state !== undefined) write(value.state);
      if (value.image) await emitImage(value.image);
    }
    const screenshot = value.image ? decode(value.image.data) : undefined;
    return kind === 'ax'
      ? value.state
      : kind === 'screenshot'
        ? screenshot
        : { state: value.state, screenshot };
  };
  const action = (kind, args) =>
    invoke('action', { handle, action: { kind, ...args } }).then((result) => {
      if (result?.warning) write(result.warning);
    });
  const target = {
    getAXState: (options) => observe('ax', options),
    getScreenshot: (options) => observe('screenshot', options),
    getAXStateAndScreenshot: (options) => observe('both', options),
    click: (target, options = {}) => action('click', { target, options }),
    drag: (from, to) => action('drag', { from, to }),
    scroll: (target, direction, distance) => action('scroll', { target, direction, distance }),
    setValue: (index, value) => action('setValue', { index, value }),
    selectText: (index, text, options = {}) => action('selectText', { index, text, options }),
    performSecondaryAction: (index, action) => action('secondary', { index, action }),
  };
  if (result.kind === 'tab') {
    Object.assign(target, {
      id: result.id,
      typeText: (index, text) => action('typeText', { index, text }),
      paste: (index, text, options = {}) => action('paste', { index, text, options }),
      pressKey: (index, key) => action('pressKey', { index, key }),
      goto: (url) => action('navigate', { url }),
      back: () => action('back', {}),
      forward: () => action('forward', {}),
      reload: () => action('reload', {}),
      close: () => action('close', {}),
    });
  } else {
    Object.assign(target, {
      typeText: (text) => action('typeText', { text }),
      paste: (text, options = {}) => action('paste', { text, options }),
      pressKey: (key) => action('pressKey', { key }),
    });
  }
  write(result.state);
  return Object.freeze(target);
};
Object.defineProperty(globalThis, 'nodeRepl', { value: Object.freeze({ write, emitImage }) });
Object.defineProperty(globalThis, 'cua', {
  value: Object.freeze({
    computer: Object.freeze({ target: nativePlatform }),
    getState: (options) => observation('getState', options),
    listApps: (options) => observation('listApps', options),
    listWindows: (options) => observation('listWindows', options),
    getApp: (target) => bind('getApp', { target }),
    listBrowsers: (options) => observation('listBrowsers', options),
    listTabs: (options) => observation('listTabs', options),
    getBrowser: async (options = {}) => {
      const result = await invoke('getBrowser', { options });
      await documentation();
      return Object.freeze({ browserId: result.id, documentation });
    },
    getTab: (reference, options = {}) => bind('getTab', { reference, options }),
    createBrowserTab: (browserId, url = 'about:blank', options = {}) =>
      bind('createBrowserTab', { browserId, url, options }),
    rewriteDocumentation: async () => {
      documented = false;
      return documentation();
    },
  }),
});
