-- SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
--
-- SPDX-License-Identifier: EUPL-1.2

local instance_key = KEYS[1]
local instance_metrics_key = KEYS[2]
local instance_resource_key = KEYS[3]
local service_kind_key = KEYS[4]
local orchestrator_services_key = KEYS[5]

local service_id = ARGV[1]
local url = ARGV[2]
local kind = ARGV[3]
local key_ids = ARGV[4]
local load = ARGV[5]
local accepting_jobs = ARGV[6]
local managed_resources = ARGV[7]
local resource_keys = ARGV[8]

-- Check if a service instance with the same service id already exists

if redis.call("EXISTS", instance_key) == 1 then
    return 1 -- service instance already exists
end

-- Set resource keys
local resources = cjson.decode(managed_resources)
local resource_lookup_keys = cjson.decode(resource_keys)
local created = { instance_resource_key }

for i, resource in ipairs(resources) do
    if not redis.call("SET", resource_lookup_keys[i], service_id, "NX") then
        -- cleanup keys that were already set
        for j = 1, #created do
            redis.call("DEL", created[j])
        end
        return 2 -- resource already exists
    end
    created[#created + 1] = resource_lookup_keys[i]
    redis.call("SADD", instance_resource_key, resource)
end

-- Set service data
redis.call("HSET", instance_key,
            "url", url,
            "kind", kind,
            "api_key_ids", key_ids)

-- Set metrics
redis.call("HSET", instance_metrics_key,
            "load", load,
            "accepting_jobs", accepting_jobs)

-- Add to service set
redis.call("SADD", service_kind_key, service_id)

-- Track which orchestrator owns this service (used for cleanup on crash)
redis.call("SADD", orchestrator_services_key, service_id)

return 0
