-- 仅在隔离验收项目、且经用户确认后运行。创建合成账号，最后 ROLLBACK，不接触真实笔记。
begin;
insert into auth.users(id,email) values
 ('00000000-0000-4000-8000-000000000001','qj-synthetic-a@example.invalid'),
 ('00000000-0000-4000-8000-000000000002','qj-synthetic-b@example.invalid');
set local role authenticated;
select set_config('request.jwt.claim.sub','00000000-0000-4000-8000-000000000001',true);
do $$
declare
  d jsonb:=jsonb_build_object('id','00000000-0000-4000-8000-000000000010','kind','note','title','合成标题','body','合成正文','body_json',null,'created_at','2026-10-10T00:00:00Z','updated_at','2026-10-10T00:00:00Z','deleted_at',null,'is_pinned',false,'sort_order',0,'purged',false);
  a jsonb; b jsonb;
begin
  a:=public.qj_push('00000000-0000-4000-8000-000000000101',0,d);
  if a->>'ok'<>'true' or a#>>'{item,version}'<>'1' then raise exception 'create failed'; end if;
  b:=public.qj_push('00000000-0000-4000-8000-000000000101',0,d);
  if a<>b then raise exception 'idempotent retry failed'; end if;
  d:=d||jsonb_build_object('body','第二个版本');
  a:=public.qj_push('00000000-0000-4000-8000-000000000102',1,d);
  if a#>>'{item,version}'<>'2' then raise exception 'update failed'; end if;
  b:=public.qj_push('00000000-0000-4000-8000-000000000103',1,d||jsonb_build_object('body','并发旧版本'));
  if b->>'ok'<>'false' or b#>>'{item,document,body}'<>'第二个版本' then raise exception 'CAS failed'; end if;
  begin
    perform public.qj_push('00000000-0000-4000-8000-000000000102',1,d||jsonb_build_object('body','修改操作载荷'));
    raise exception 'operation reuse unexpectedly accepted';
  exception when raise_exception then
    if sqlerrm='operation reuse unexpectedly accepted' then raise; end if;
  end;
  begin
    update public.qj_documents set version=99;
    raise exception 'direct update unexpectedly accepted';
  exception when insufficient_privilege then null; end;
  begin
    perform * from public.qj_receipts;
    raise exception 'receipts unexpectedly readable';
  exception when insufficient_privilege then null; end;
  a:=public.qj_push('00000000-0000-4000-8000-000000000104',2,d||jsonb_build_object('purged',true,'deleted_at','2026-10-10T01:00:00Z'));
  if a#>>'{item,document,body}'<>'' then raise exception 'purge did not clear content'; end if;
  b:=public.qj_push('00000000-0000-4000-8000-000000000105',3,d);
  if b->>'ok'<>'false' or b#>>'{item,document,purged}'<>'true' then raise exception 'tombstone resurrected'; end if;
end $$;
select set_config('request.jwt.claim.sub','00000000-0000-4000-8000-000000000002',true);
do $$ begin
  if exists(select 1 from public.qj_documents) then raise exception 'account B can read account A'; end if;
end $$;
reset role;
rollback;
-- 只有前面的所有断言执行成功并完成回滚，才会返回此明确结果。
select 'PASS: all synthetic permission, retry, CAS and tombstone checks completed; test data rolled back' as acceptance_result;
