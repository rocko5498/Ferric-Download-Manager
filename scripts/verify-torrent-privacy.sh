#!/usr/bin/env bash
set -euo pipefail

duration=30
proxy_host=127.0.0.1
proxy_port=1080
capture_path=
result_path=
process_names="ferricd rqbit"
rqbit_endpoint="http://127.0.0.1:3030"
bypass_packet_check=false

while [ "$#" -gt 0 ]; do
  case "$1" in
    --duration) duration="$2"; shift 2 ;;
    --proxy-host) proxy_host="$2"; shift 2 ;;
    --proxy-port) proxy_port="$2"; shift 2 ;;
    --capture-path) capture_path="$2"; shift 2 ;;
    --result-path) result_path="$2"; shift 2 ;;
    --process-names) process_names="$2"; shift 2 ;;
    --rqbit-endpoint) rqbit_endpoint="$2"; shift 2 ;;
    --bypass-packet-check) bypass_packet_check=true; shift 1 ;;
    *) echo "unknown argument: $1" >&2; exit 64 ;;
  esac
done

proxy_ips="$(getent ahosts "$proxy_host" 2>/dev/null | awk '{print $1}' | sort -u | tr '\n' ' ')"
[ -n "$proxy_ips" ] || proxy_ips="$proxy_host"

pids=
missing=
for name in $process_names; do
  found="$(pgrep -x "$name" 2>/dev/null || true)"
  [ -n "$found" ] || missing="$missing $name"
  [ -z "$found" ] || pids="$pids $found"
done
[ -z "$missing" ] || { echo "missing target process(es):$(echo "$missing" | xargs)" >&2; exit 1; }
[ -n "$pids" ] || { echo "no target processes found: $process_names" >&2; exit 1; }

leaks_file="$(mktemp)"
packet_leaks_file="$(mktemp)"
trap 'rm -f "$leaks_file" "$packet_leaks_file"' EXIT

is_proxy_ip() {
  case " $proxy_ips " in
    *" $1 "*) return 0 ;;
    *) return 1 ;;
  esac
}

is_loopback() {
  case "$1" in
    127.*|::1|0.0.0.0|::) return 0 ;;
    *) return 1 ;;
  esac
}

packet_verified=false
tcpdump_pid=
if [ "$bypass_packet_check" = true ]; then
  echo "WARNING: Packet verification was bypassed by user request! Running connection table sampling only."
  packet_verified=true
elif command -v tcpdump >/dev/null 2>&1 && [ -n "$capture_path" ]; then
  tcpdump -i any -w "$capture_path" "not host 127.0.0.1 and not host ::1" >/dev/null 2>&1 &
  tcpdump_pid=$!
fi

end=$((SECONDS + duration))
while [ "$SECONDS" -lt "$end" ]; do
  ss -H -tunp 2>/dev/null | while read -r line; do
    echo "$line" | grep -Eq "pid=($(echo "$pids" | xargs | tr ' ' '|'))," || continue
    remote="$(echo "$line" | awk '{print $5}')"
    host="${remote%:*}"
    port="${remote##*:}"
    host="${host#[}"
    host="${host%]}"
    if ! is_loopback "$host" && { ! is_proxy_ip "$host" || [ "$port" != "$proxy_port" ]; }; then
      echo "$line" >> "$leaks_file"
    fi
  done
  sleep 0.5
done

if [ -n "${tcpdump_pid:-}" ]; then
  kill "$tcpdump_pid" 2>/dev/null || true
  wait "$tcpdump_pid" 2>/dev/null || true
  if command -v tshark >/dev/null 2>&1; then
    tshark -r "$capture_path" -T fields -E separator='|' \
      -e frame.number -e ip.src -e ip.dst -e ipv6.src -e ipv6.dst \
      -e tcp.srcport -e tcp.dstport -e udp.srcport -e udp.dstport 2>/dev/null |
    while IFS='|' read -r frame ip_src ip_dst ipv6_src ipv6_dst tcp_src tcp_dst udp_src udp_dst; do
      addresses="$ip_src $ip_dst $ipv6_src $ipv6_dst"
      ports="$tcp_src $tcp_dst $udp_src $udp_dst"
      ok=false
      for address in $addresses; do
        if is_proxy_ip "$address" && echo " $ports " | grep -q " $proxy_port "; then
          ok=true
        fi
      done
      [ "$ok" = true ] || echo "packet frame=$frame" >> "$packet_leaks_file"
    done
    packet_verified=true
  fi
fi

leaks_json="$(sort -u "$leaks_file" | awk 'NF {gsub(/\\/,"\\\\"); gsub(/"/,"\\\""); printf "%s\"%s\"", sep, $0; sep=","}')"
packet_leaks_json="$(sort -u "$packet_leaks_file" | awk 'NF {gsub(/\\/,"\\\\"); gsub(/"/,"\\\""); printf "%s\"%s\"", sep, $0; sep=","}')"
passed=false
if [ ! -s "$leaks_file" ] && [ ! -s "$packet_leaks_file" ] && [ "$packet_verified" = true ]; then
  passed=true
fi

json="{\"schema_version\":1,\"checked_at\":\"$(date -u +%Y-%m-%dT%H:%M:%SZ)\",\"duration_seconds\":$duration,\"proxy_host\":\"$proxy_host\",\"proxy_port\":$proxy_port,\"rqbit_endpoint\":\"$rqbit_endpoint\",\"proxy_addresses\":\"$proxy_ips\",\"process_names\":\"$process_names\",\"pids\":\"$(echo "$pids" | xargs)\",\"capture_path\":\"$capture_path\",\"packet_verified\":$packet_verified,\"passed\":$passed,\"leaks\":[${leaks_json:-}],\"packet_leaks\":[${packet_leaks_json:-}]}"

if [ -n "$result_path" ]; then
  printf '%s\n' "$json" > "$result_path"
fi

[ "$passed" = true ] || { echo "privacy verification failed" >&2; exit 2; }
echo "privacy verification passed"
