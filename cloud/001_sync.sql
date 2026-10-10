-- 仅在用户确认的 Supabase 免费项目中执行。客户端绝不持有 service_role 或发信凭据。
begin;
create table public.qj_documents (
  owner uuid not null references auth.users(id) on delete cascade,
  id uuid not null,
  version bigint not null check(version>0),
  document jsonb not null,
  primary key(owner,id)
);
create table public.qj_receipts (
  owner uuid not null references auth.users(id) on delete cascade,
  op uuid not null,
  request_hash text not null,
  response jsonb not null,
  primary key(owner,op)
);
alter table public.qj_documents enable row level security;
alter table public.qj_receipts enable row level security;
create policy own_documents on public.qj_documents for select to authenticated using(owner=auth.uid());
-- 禁止 REST 直接写入；所有写入统一经过比较版本的 RPC。回执不向客户端开放。
revoke all on public.qj_documents, public.qj_receipts from anon,authenticated;
grant select on public.qj_documents to authenticated;

create function public.qj_push(p_op uuid,p_base bigint,p_document jsonb) returns jsonb
language plpgsql security definer set search_path='' as $$
declare
  uid uuid:=auth.uid(); item_id uuid; current_row public.qj_documents;
  receipt public.qj_receipts; result jsonb;
  req jsonb:=jsonb_build_object('base',p_base,'document',p_document);
  req_hash text;
begin
  if uid is null then raise exception 'authentication required' using errcode='42501'; end if;
  item_id:=(p_document->>'id')::uuid;
  if item_id is null or p_base is null or p_base<0 or p_op is null
    or jsonb_typeof(p_document) is distinct from 'object'
    or (p_document->>'kind') not in ('sticky','note') or not (p_document ?& array['kind','title','body','body_json','created_at','updated_at','deleted_at','is_pinned','sort_order','purged'])
    or jsonb_typeof(p_document->'kind') is distinct from 'string'
    or jsonb_typeof(p_document->'title') is distinct from 'string'
    or jsonb_typeof(p_document->'body') is distinct from 'string'
    or jsonb_typeof(p_document->'body_json') not in ('string','null')
    or jsonb_typeof(p_document->'created_at') is distinct from 'string'
    or jsonb_typeof(p_document->'updated_at') is distinct from 'string'
    or jsonb_typeof(p_document->'deleted_at') not in ('string','null')
    or jsonb_typeof(p_document->'is_pinned') is distinct from 'boolean'
    or jsonb_typeof(p_document->'purged') is distinct from 'boolean'
    or jsonb_typeof(p_document->'sort_order') is distinct from 'number'
    or length(p_document->>'title')>100000 or octet_length(p_document::text)>4200000
    or (p_document - array['id','kind','title','body','body_json','created_at','updated_at','deleted_at','is_pinned','sort_order','purged']) <> '{}'::jsonb
  then raise exception 'invalid document'; end if;
  perform (p_document->>'created_at')::timestamptz, (p_document->>'updated_at')::timestamptz;
  if p_document->>'deleted_at' is not null then perform (p_document->>'deleted_at')::timestamptz; end if;
  req_hash:=encode(sha256(convert_to(req::text,'UTF8')),'hex');
  if (p_document->>'purged')::boolean then
    p_document:=p_document||jsonb_build_object('title','','body','','body_json',null);
  end if;
  -- 账号级锁将重试和并发写入串行化；不使用设备时间，也不把本地 revision 当云版本。
  perform pg_advisory_xact_lock(hashtextextended(uid::text,0));
  select * into receipt from public.qj_receipts where owner=uid and op=p_op;
  if found then
    if receipt.request_hash<>req_hash then raise exception 'operation reused with different payload'; end if;
    if (receipt.response->>'ok')::boolean then
      return jsonb_build_object('ok',true,'item',jsonb_build_object('version',receipt.response->'version','document',p_document));
    end if;
    select * into current_row from public.qj_documents where owner=uid and id=item_id;
    return jsonb_build_object('ok',false,'item',jsonb_build_object('version',current_row.version,'document',current_row.document));
  end if;
  select * into current_row from public.qj_documents where owner=uid and id=item_id for update;
  if not found then
    if p_base<>0 then raise exception 'missing remote version'; end if;
    insert into public.qj_documents values(uid,item_id,1,p_document) returning * into current_row;
    result:=jsonb_build_object('ok',true,'item',jsonb_build_object('version',current_row.version,'document',current_row.document));
  elsif current_row.version=p_base then
    -- 永久删除后同一标识永不复活；用户恢复内容需明确另存为新条目。
    if (current_row.document->>'purged')::boolean then
      result:=jsonb_build_object('ok',false,'item',jsonb_build_object('version',current_row.version,'document',current_row.document));
    else
      update public.qj_documents set version=version+1,document=p_document where owner=uid and id=item_id returning * into current_row;
      result:=jsonb_build_object('ok',true,'item',jsonb_build_object('version',current_row.version,'document',current_row.document));
    end if;
  else
    result:=jsonb_build_object('ok',false,'item',jsonb_build_object('version',current_row.version,'document',current_row.document));
  end if;
  -- 回执只保留请求摘要与版本，不积累历史正文；永久删除清空正文并永久保留墓碑。
  insert into public.qj_receipts values(uid,p_op,req_hash,jsonb_build_object('ok',result->'ok','version',result#>'{item,version}'));
  return result;
end $$;
revoke all on function public.qj_push(uuid,bigint,jsonb) from public,anon;
grant execute on function public.qj_push(uuid,bigint,jsonb) to authenticated;
commit;
