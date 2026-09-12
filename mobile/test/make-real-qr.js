// 生成真实网关配对链接的全屏二维码页（手机扫码用）。
const QR = require('E:/develop/dsh-workspace/qianxun/node_modules/qrcode');
const fs = require('node:fs');

const url =
  'http://192.168.20.2:23090/qx-gate?token=' +
  'e444322c49188ef225341f372358b2bdaf522e4b89c6c47e4dbad83e479a0388';

QR.toDataURL(url, { width: 900, margin: 2 }).then(dataUrl => {
  const html =
    '<!doctype html><html><head><meta charset="utf-8"><title>千寻配对</title></head>' +
    '<body style="margin:0;background:#fff;display:grid;place-items:center;height:100vh">' +
    '<img src="' +
    dataUrl +
    '" style="width:min(92vmin,860px)">' +
    '</body></html>';
  fs.writeFileSync(__dirname + '/pair-qr-real.html', html);
  console.log('ok ->', __dirname + '/pair-qr-real.html');
});
