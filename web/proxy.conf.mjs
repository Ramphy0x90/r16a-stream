// Dev server proxy for ng serve. Web, API and media share one origin, as in production, which
// the session cookie and the CSP need. No CORS anywhere (SPEC 18.1).
//
// A module rather than proxy.conf.json, because the bucket paths must have Cookie and
// Authorization removed before Garage, like Traefik does, and the JSON format can only set
// headers, not remove them (DECISIONS 061).
//
// The Host header is kept, since presigned URLs are signed for http://localhost:4200.

const api = process.env.R16A_DEV_API ?? 'http://127.0.0.1:8081';
const garage = process.env.R16A_DEV_GARAGE ?? 'http://127.0.0.1:3900';

/** Removes credentials from proxied bucket requests. */
function stripCredentials(proxy) {
  proxy.on('proxyReq', (proxyReq) => {
    proxyReq.removeHeader('cookie');
    proxyReq.removeHeader('authorization');
  });
}

export default {
  '/api': { target: api, secure: false, changeOrigin: false },
  '/r16a-stream-media': { target: garage, secure: false, changeOrigin: false, configure: stripCredentials },
  '/r16a-stream-uploads': { target: garage, secure: false, changeOrigin: false, configure: stripCredentials },
};
