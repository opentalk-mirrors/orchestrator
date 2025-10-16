#!/usr/bin/env bash

UUID=$1
if [[ -z "$UUID" ]]; then
  UUID=`cat /proc/sys/kernel/random/uuid`
  echo "No uuid given as argument, use random uuid: $UUID"
fi


curl -v -X PUT \
  -H "Content-Type: application/json" \
  -d @- \
  "http://localhost:3000/roomserver/rooms/$UUID" <<'EOF'
{
  "created_by": {
    "id": "08443728-2330-477a-a8a1-519e932c4f14",
    "email": "daniel@kerkmann.dev",
    "title": "",
    "firstname": "Daniél",
    "lastname": "Kerkmann",
    "display_name": "Daniél Kerkmann",
    "avatar_url": "",
    "timezone": "Europe/Berlin"
  },
  "waiting_room": true,
  "tariff": {
    "id": "4cad2e42-5dd0-4902-b7ce-e05736a35cb6",
    "name": "Some Tariff",
    "quotas": {},
    "modules": {}
  },
  "streaming_links": [],
  "e2e_encryption": false
}
EOF
