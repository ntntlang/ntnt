-- Private Redis storage protocol. Every potential WRONGTYPE/metadata decode is
-- validated before the first write: Redis Lua errors do NOT roll back writes.
local a=cjson.decode(ARGV[1])
local ns='__ntnt:jobs-retention:v1:'
local all,good,bad,meta,state,queue=ns..'all',ns..'good',ns..'bad',ns..'records',ns..'state',ns..'backfill'
local function check(k,t)
  local actual=redis.call('TYPE',k).ok
  if actual~='none' and actual~=t then error('retention key type conflict') end
end
for _,k in ipairs({all,good,bad}) do check(k,'zset') end
for _,k in ipairs({meta,state}) do check(k,'hash') end
check(queue,'list')
local function number(v)
  local n=tonumber(v or '0')
  if not n or n<0 or n>9007199254740991 or n~=math.floor(n) then error('retention accounting invalid') end
  return n
end
local bytes=number(redis.call('HGET',state,'bytes'))
local count=redis.call('ZCARD',all)
local function get(k)
  check(k,'string');check(k..':__type','string')
  return redis.call('GET',k),redis.call('GET',k..':__type')
end
local function decode(raw,hint)
  if not raw then return nil end
  local ok,m=pcall(cjson.decode,raw)
  if not ok or type(m)~='table' then return nil end
  if m.__ntnt_t=='map' and type(m.v)=='table' then return m.v end
  if hint=='map' then return m end
  return nil
end
local function terminal(k,raw,hint)
  local m=decode(raw,hint)
  if not m or type(m.id)~='string' or k~='jobs:data:'..m.id then return nil end
  local cat,field
  if m.status=='completed' then cat=0;field='completed_at'
  elseif m.status=='cancelled' then cat=0;field='cancelled_at'
  elseif m.status=='dead' then cat=1;field='dead_at'
  elseif m.status=='failed' then cat=1;field='failed_at'
  elseif m.status=='expired' then cat=1;field='expired_at'
  else return nil end
  local t=a.now
  local ts=m._retention_terminal_at or m[field]
  if type(ts)=='string' and string.match(ts,'^%d+$') then
    local ms=string.len(ts)>6 and tonumber(string.sub(ts,1,-7)) or 0
    if ms and ms>=0 and ms<=a.now then t=ms end
  end
  return {category=cat,time=t,bytes=string.len(raw),fingerprint=redis.sha1hex(raw..'\0'..(hint or ''))}
end
local function metadata(k)
  local raw=redis.call('HGET',meta,k)
  if not raw then return nil end
  local ok,t=pcall(cjson.decode,raw)
  if not ok or type(t)~='table' or (t.category~=0 and t.category~=1) or type(t.fingerprint)~='string' then error('retention metadata invalid') end
  number(t.time);number(t.bytes)
  return t
end
local changes,related={},{}
local function index(k,t)
  local previous=changes[k]
  if previous==nil then previous=metadata(k) end
  if previous then count=count-1;bytes=bytes-previous.bytes end
  if t then
    if previous and previous.fingerprint==t.fingerprint then t.time=previous.time end
    count=count+1;bytes=bytes+t.bytes
  end
  changes[k]=t or false
end
local function pending(m)
  if not m or type(m.id)~='string' or type(m.pending_key)~='string' then return '' end
  local pri,ts,id=string.match(m.pending_key,'^jobs:pending:(%d%d):(%d+):([^:]+)$')
  if id==m.id then return m.pending_key end
  return ''
end
local function callback(m,creating)
  if not m or m.type~='_BatchCallback' or type(m.payload)~='table' then return end
  local b,t=m.payload.batch_id,m.payload.callback_type
  if type(b)~='string' or (t~='on_success' and t~='on_complete' and t~='on_death') or m.id~='cb-'..b..'-'..t then return end
  local guard=ns..'callback:'..b..':'..t
  check(guard,'string')
  if creating and redis.call('EXISTS',guard)==1 then error('callback already enqueued') end
  local batch='jobs:batch:'..b
  check(batch,'string')
  local ttl=redis.call('PTTL',batch)
  if ttl>=0 then table.insert(related,{'SET',guard,'{"__ntnt_t":"bool","v":true}','PX',math.max(ttl,1)})
  elseif ttl==-1 then table.insert(related,{'SET',guard,'{"__ntnt_t":"bool","v":true}'})
  elseif creating then error('callback batch no longer exists') end
end
local function plan_related(k,old,new,prune)
  if prune then callback(old,false) end
  local m=old
  if m and k=='jobs:data:'..(m.id or '') then
    local pk=pending(m)
    if pk~='' then
      check(pk,'string')
      if redis.call('GET',pk)==m.id then table.insert(related,{'DEL',pk}) end
    end
    if prune and type(m.dedup_key)=='string' and string.sub(m.dedup_key,1,12)=='jobs:unique:' then
      local dk=m.dedup_key
      check(dk,'string')
      if redis.call('GET',dk)==m.id then
        table.insert(related,{'SET',dk,cjson.encode({__ntnt_t='map',v={__ntnt_retired_job=m.id}}),'KEEPTTL'})
      end
    end
  end
  if new and k=='jobs:data:'..(new.id or '') then
    local ak='jobs:active:'..new.id
    check(ak,'string')
    if new.status=='active' then table.insert(related,{'SET',ak,new.id,'EX',300})
    elseif redis.call('GET',ak)==new.id then table.insert(related,{'DEL',ak}) end
    if (new.status=='dead' or new.status=='cancelled' or new.status=='expired' or new.status=='failed') and type(new.dedup_key)=='string' and string.sub(new.dedup_key,1,12)=='jobs:unique:' then
      check(new.dedup_key,'string')
      if redis.call('GET',new.dedup_key)==new.id then table.insert(related,{'DEL',new.dedup_key}) end
    end
  end
  if new and k=='jobs:data:'..(new.id or '') and (new.status=='pending'  or new.status=='scheduled' or new.status=='retrying') then
    local pk=pending(new)
    if pk~='' then
      check(pk,'string')
      local owner=redis.call('GET',pk)
      if owner and owner~=new.id then error('retention pending owner conflict') end
      table.insert(related,{'SET',pk,new.id})
    end
  end
end
if a.op=='batch_meta' then
  check(a.key,'string')
  for _,t in ipairs({'on_success','on_complete','on_death'}) do check(ns..'callback:'..a.batch..':'..t,'string') end
  if a.ttl==cjson.null then redis.call('SET',a.key,a.raw)
  else redis.call('SET',a.key,a.raw,'EX',a.ttl) end
  for _,t in ipairs({'on_success','on_complete','on_death'}) do
    local k=ns..'callback:'..a.batch..':'..t
    if a.ttl==cjson.null then redis.call('PERSIST',k) else redis.call('EXPIRE',k,a.ttl) end
  end
  return '0'
end
if a.op=='release_unique' then
  check(a.key,'string')
  local raw,hint=get('jobs:data:'..a.id)
  local m=decode(raw,hint)
  if m and (m.status=='dead' or m.status=='cancelled' or m.status=='expired' or m.status=='failed') and redis.call('GET',a.key)==a.id then redis.call('DEL',a.key) end
  return '0'
end
if a.op=='snapshot' then
  local raw,hint=get(a.key)
  return cjson.encode({raw=raw or cjson.null,hint=hint or cjson.null})
end
if a.op=='policy' then
  local old=redis.call('HGET',state,'policy')
  if a.replace or not old then
    old=cjson.encode(a.policy)
    redis.call('HSET',state,'policy',old)
  end
  return old
end
if a.op=='has_work' then
  local p=a.policy
  if not p.enabled then return '0' end
  if redis.call('HGET',state,'complete')~='1' or redis.call('LLEN',queue)>0 or count>p.max_records or bytes>p.max_bytes then return '1' end
  for cat,z in ipairs({good,bad}) do
    local days=cat==1 and p.completed_days or p.failed_days
    if #redis.call('ZRANGEBYSCORE',z,'-inf',a.now-days*86400000,'LIMIT',0,1)>0 then return '1' end
  end
  return '0'
end
local writes={}
local deleted=0
local page,overflow,newcursor,done=nil,nil,nil,nil
if a.op=='change' or a.op=='remove' then
  local raw,hint=get(a.key)
  local expected=a.expected
  if expected==cjson.null then
    if raw then return '-1' end
  elseif raw~=expected.raw or (hint or '')~=expected.kind then return '-1' end
  local replacement=a.op=='change' and a.raw or false
  if replacement and not raw then callback(decode(replacement,nil),true) end
  index(a.key,replacement and terminal(a.key,replacement,nil) or nil)
  plan_related(a.key,decode(raw,hint),replacement and decode(replacement,nil) or nil,a.op=='remove')
  table.insert(writes,replacement and {'SET',a.key,replacement} or {'DEL',a.key})
  table.insert(writes,{'DEL',a.key..':__type'})
elseif a.op=='maintain' then
  local p=a.policy
  local stored=redis.call('HGET',state,'policy')
  if stored then
    stored=cjson.decode(stored)
    if type(stored)~='table' then error('retention policy invalid') end
    for k,v in pairs(p) do if stored[k]~=v then return '0' end end
    for k in pairs(stored) do if p[k]==nil then return '0' end end
  end
  if not p.enabled then return '0' end
  local n=p.batch_size
  local cursor=redis.call('HGET',state,'cursor') or '0'
  done=redis.call('HGET',state,'complete')=='1'
  page=redis.call('LRANGE',queue,0,n-1)
  if #page==0 and not done then
    -- COUNT is a server hint, never a strict response-size/latency bound.
    -- Overshoot is durably queued; only n records are inspected in this pass.
    local scan=redis.call('SCAN',cursor,'MATCH','jobs:data:*','COUNT',n)
    newcursor=scan[1];done=newcursor=='0';overflow={}
    for i,k in ipairs(scan[2]) do
      if i<=n then table.insert(page,k) else table.insert(overflow,k) end
    end
  end
  local candidates,seen={},{}
  local function candidate(k,t)
    if t and not seen[k] then seen[k]=true;table.insert(candidates,{key=k,time=t.time}) end
  end
  for _,k in ipairs(page) do
    local raw,hint=get(k)
    index(k,terminal(k,raw,hint))
    candidate(k,changes[k])
  end
  local indexed_complete=done and ((newcursor and #overflow==0) or (not newcursor and redis.call('LLEN',queue)<=#page))
  local function pressure() return indexed_complete and (count>p.max_records or bytes>p.max_bytes) end
  if pressure() then
    for _,k in ipairs(redis.call('ZRANGE',all,0,n-1)) do candidate(k,metadata(k)) end
  else
    for cat,z in ipairs({good,bad}) do
      local days=cat==1 and p.completed_days or p.failed_days
      for _,k in ipairs(redis.call('ZRANGEBYSCORE',z,'-inf',a.now-days*86400000,'LIMIT',0,n)) do candidate(k,metadata(k)) end
    end
  end
  table.sort(candidates,function(x,y) if x.time==y.time then return x.key<y.key else return x.time<y.time end end)
  local examined=0
  for _,entry in ipairs(candidates) do
    if examined>=n then break end
    local k=entry.key
    local t=changes[k];if t==nil then t=metadata(k) end
    if t then
      local days=t.category==0 and p.completed_days or p.failed_days
      if t.time<=a.now-days*86400000 or pressure() then
        examined=examined+1
        local raw,hint=get(k)
        local current=terminal(k,raw,hint)
        if current and current.fingerprint==t.fingerprint then
          plan_related(k,decode(raw,hint),nil,true)
          table.insert(writes,{'DEL',k,k..':__type'})
          index(k,nil);deleted=deleted+1
        else index(k,current) end
      end
    end
  end
else error('unknown retention operation') end
-- All validation completed. No commands below can hit WRONGTYPE, JSON errors,
-- or integer overflow with the validated metadata. Redis OOM/server failure is
-- still an operational failure, not a distributed durability guarantee.
number(bytes);number(count)
for _,cmd in ipairs(related) do redis.call(unpack(cmd)) end
for _,cmd in ipairs(writes) do redis.call(unpack(cmd)) end
for k,t in pairs(changes) do
  redis.call('ZREM',all,k);redis.call('ZREM',good,k);redis.call('ZREM',bad,k)
  if t then
    redis.call('ZADD',all,t.time,k);redis.call('ZADD',t.category==0 and good or bad,t.time,k)
    redis.call('HSET',meta,k,cjson.encode(t))
  else redis.call('HDEL',meta,k) end
end
redis.call('HSET',state,'bytes',string.format('%.0f',bytes))
if page then
  if newcursor then
    for _,k in ipairs(overflow) do redis.call('RPUSH',queue,k) end
    redis.call('HSET',state,'cursor',newcursor,'complete',done and '1' or '0')
  else redis.call('LTRIM',queue,#page,-1) end
end
return tostring(deleted)
