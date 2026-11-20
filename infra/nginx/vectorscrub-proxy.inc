# Shared proxy settings. Install as /etc/nginx/vectorscrub-proxy.inc.
proxy_http_version 1.1;
proxy_set_header Connection "";
proxy_set_header Host              $host;
proxy_set_header X-Real-IP         $remote_addr;
# OVERWRITE, never append ($proxy_add_x_forwarded_for would keep what the client sent). The
# backend trusts only the entries our own proxies add; a client-supplied value must not survive.
proxy_set_header X-Forwarded-For   $remote_addr;
proxy_set_header X-Forwarded-Proto $scheme;
proxy_connect_timeout 5s;
proxy_send_timeout    35s;
proxy_read_timeout    35s;
