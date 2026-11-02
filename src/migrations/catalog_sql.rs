//! Fixed, pg_catalog-qualified introspection only. Never prepare application SQL
//! on the target, evaluate a stored expression, or select application rows.
pub(super) const QUERIES: &[(&str, &str)] = &[
    (
        "schema",
        r#"
SELECT n.nspname::text, jsonb_build_object('owner',pg_get_userbyid(n.nspowner),
 'grants',(SELECT jsonb_agg(jsonb_build_object('grantor',pg_get_userbyid(a.grantor),'grantee',CASE WHEN a.grantee=0 THEN jsonb_build_object('public',true) ELSE to_jsonb(pg_get_userbyid(a.grantee)) END,'privilege',a.privilege_type,'grantable',a.is_grantable) ORDER BY a.grantor,a.grantee,a.privilege_type)
 FROM aclexplode(coalesce(n.nspacl,acldefault('n',n.nspowner))) a)),
 has_schema_privilege(n.oid,'USAGE')
FROM pg_catalog.pg_namespace n WHERE n.nspname=$1
"#,
    ),
    (
        "relation",
        r#"
SELECT c.relname::text, jsonb_build_object('kind',c.relkind,'owner',pg_get_userbyid(c.relowner),
 'persistence',c.relpersistence,'row_security',c.relrowsecurity,'force_row_security',c.relforcerowsecurity,
 'partition',c.relispartition,'replica_identity',c.relreplident,'access_method',am.amname,
 'options',c.reloptions,'tablespace',ts.spcname,'comment',obj_description(c.oid,'pg_class'),
 'partition_key',CASE WHEN c.relkind='p' THEN pg_get_partkeydef(c.oid) END,'partition_bound',pg_get_expr(c.relpartbound,c.oid),
 'partition_of',(SELECT pc.relname FROM pg_catalog.pg_inherits h JOIN pg_catalog.pg_class pc ON pc.oid=h.inhparent WHERE h.inhrelid=c.oid AND c.relispartition),
 'grants',CASE WHEN c.relkind IN ('i','I') THEN NULL ELSE (SELECT jsonb_agg(jsonb_build_object('grantor',pg_get_userbyid(a.grantor),'grantee',CASE WHEN a.grantee=0 THEN jsonb_build_object('public',true) ELSE to_jsonb(pg_get_userbyid(a.grantee)) END,'privilege',a.privilege_type,'grantable',a.is_grantable) ORDER BY a.grantor,a.grantee,a.privilege_type)
 FROM aclexplode(coalesce(c.relacl,acldefault(CASE WHEN c.relkind='S' THEN 's'::"char" ELSE 'r'::"char" END,c.relowner))) a) END),
 ((c.relkind='r' AND am.amname='heap') OR (c.relkind IN ('i','I') AND am.oid<16384) OR c.relkind IN ('p','v','S'))
 AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_inherits h JOIN pg_catalog.pg_class ch ON ch.oid=h.inhrelid JOIN pg_catalog.pg_class pa ON pa.oid=h.inhparent
   WHERE (h.inhrelid=c.oid OR h.inhparent=c.oid) AND (NOT ch.relispartition OR ch.relnamespace<>c.relnamespace OR pa.relnamespace<>c.relnamespace))
 AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_depend WHERE classid='pg_catalog.pg_class'::regclass AND objid=c.oid AND deptype='e')
FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
LEFT JOIN pg_catalog.pg_am am ON am.oid=c.relam LEFT JOIN pg_catalog.pg_tablespace ts ON ts.oid=c.reltablespace
WHERE n.nspname=$1 ORDER BY c.relname LIMIT 10001
"#,
    ),
    (
        "column",
        r#"
SELECT quote_ident(c.relname)||'.'||quote_ident(a.attname), jsonb_build_object('type',format_type(a.atttypid,a.atttypmod),
 'type_schema',tn.nspname,'dimensions',a.attndims,'ordinal',a.attnum,'not_null',a.attnotnull,
 'default',pg_get_expr(d.adbin,d.adrelid),'identity',a.attidentity,'generated',a.attgenerated,
 'collation',CASE WHEN a.attcollation=0 THEN NULL ELSE quote_ident(cn.nspname)||'.'||quote_ident(co.collname) END,
 'collation_version',co.collversion,'storage',a.attstorage,'compression',a.attcompression,
 'comment',col_description(c.oid,a.attnum),
 'grants',(SELECT jsonb_agg(jsonb_build_object('grantor',pg_get_userbyid(x.grantor),'grantee',CASE WHEN x.grantee=0 THEN jsonb_build_object('public',true) ELSE to_jsonb(pg_get_userbyid(x.grantee)) END,'privilege',x.privilege_type,'grantable',x.is_grantable) ORDER BY x.grantor,x.grantee,x.privilege_type) FROM aclexplode(a.attacl) x)),
 tn.nspname='pg_catalog' AND (a.attcollation=0 OR cn.nspname='pg_catalog') AND NOT EXISTS (
 SELECT 1 FROM pg_catalog.pg_depend dep JOIN pg_catalog.pg_proc p ON p.oid=dep.refobjid JOIN pg_catalog.pg_namespace pn ON pn.oid=p.pronamespace
 WHERE dep.classid='pg_catalog.pg_attrdef'::regclass AND dep.objid=d.oid AND dep.refclassid='pg_catalog.pg_proc'::regclass AND pn.nspname NOT IN ('pg_catalog',$1))
FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_class c ON c.oid=a.attrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
JOIN pg_catalog.pg_type t ON t.oid=a.atttypid JOIN pg_catalog.pg_namespace tn ON tn.oid=t.typnamespace
LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid=c.oid AND d.adnum=a.attnum
LEFT JOIN pg_catalog.pg_collation co ON co.oid=a.attcollation LEFT JOIN pg_catalog.pg_namespace cn ON cn.oid=co.collnamespace
WHERE n.nspname=$1 AND c.relkind NOT IN ('i','I','S') AND a.attnum>0 AND NOT a.attisdropped ORDER BY c.relname,a.attnum LIMIT 10001
"#,
    ),
    (
        "constraint",
        r#"
SELECT quote_ident(c.relname)||'.'||quote_ident(k.conname), jsonb_build_object('kind',k.contype,'definition',pg_get_constraintdef(k.oid,false),
 'validated',k.convalidated,'deferrable',k.condeferrable,'deferred',k.condeferred,'local',k.conislocal,'no_inherit',k.connoinherit,
 'enforced',pg_catalog.to_jsonb(k)->'conenforced',
 'enforcement_triggers',(SELECT jsonb_agg(jsonb_build_object('schema',tn.nspname,'table',tc.relname,'function',p.proname,'events',t.tgtype,'enabled',t.tgenabled,'deferrable',t.tgdeferrable,'deferred',t.tginitdeferred) ORDER BY tn.nspname,tc.relname,p.proname,t.tgtype,t.tgenabled)
 FROM pg_catalog.pg_trigger t JOIN pg_catalog.pg_class tc ON tc.oid=t.tgrelid JOIN pg_catalog.pg_namespace tn ON tn.oid=tc.relnamespace JOIN pg_catalog.pg_proc p ON p.oid=t.tgfoid
 WHERE t.tgconstraint=k.oid AND t.tgisinternal)),
 k.contype IN ('p','u','c','f','n','t') AND (k.confrelid=0 OR fn.nspname=$1)
 AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_depend dep JOIN pg_catalog.pg_proc p ON p.oid=dep.refobjid JOIN pg_catalog.pg_namespace pn ON pn.oid=p.pronamespace WHERE dep.classid='pg_catalog.pg_constraint'::regclass AND dep.objid=k.oid AND dep.refclassid='pg_catalog.pg_proc'::regclass AND pn.nspname NOT IN ('pg_catalog',$1))
FROM pg_catalog.pg_constraint k JOIN pg_catalog.pg_class c ON c.oid=k.conrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
LEFT JOIN pg_catalog.pg_class fc ON fc.oid=k.confrelid LEFT JOIN pg_catalog.pg_namespace fn ON fn.oid=fc.relnamespace
WHERE n.nspname=$1 ORDER BY c.relname,k.conname LIMIT 10001
"#,
    ),
    (
        "index",
        r#"
SELECT c.relname::text,jsonb_build_object('table',i.indrelid::regclass::text,'definition',pg_get_indexdef(i.indexrelid,0,false),'valid',i.indisvalid,
 'ready',i.indisready,'live',i.indislive,'unique',i.indisunique,'primary',i.indisprimary,'exclusion',i.indisexclusion,
 'immediate',i.indimmediate,'clustered',i.indisclustered,'replica_identity',i.indisreplident,'nulls_not_distinct',i.indnullsnotdistinct),
 am.oid<16384 AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_depend dep JOIN pg_catalog.pg_proc p ON p.oid=dep.refobjid JOIN pg_catalog.pg_namespace pn ON pn.oid=p.pronamespace WHERE dep.classid='pg_catalog.pg_class'::regclass AND dep.objid=i.indexrelid AND dep.refclassid='pg_catalog.pg_proc'::regclass AND pn.nspname NOT IN ('pg_catalog',$1))
 AND NOT EXISTS(SELECT 1 FROM unnest(i.indclass) x JOIN pg_catalog.pg_opclass o ON o.oid=x JOIN pg_catalog.pg_namespace onsp ON onsp.oid=o.opcnamespace WHERE onsp.nspname<>'pg_catalog')
FROM pg_catalog.pg_index i JOIN pg_catalog.pg_class c ON c.oid=i.indexrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
JOIN pg_catalog.pg_am am ON am.oid=c.relam WHERE n.nspname=$1 ORDER BY c.relname LIMIT 10001
"#,
    ),
    (
        "view",
        r#"
SELECT c.relname::text,jsonb_build_object('kind',c.relkind,'definition',pg_get_viewdef(c.oid,false)),
 c.relkind='v' AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_rewrite r JOIN pg_catalog.pg_depend dep ON dep.classid='pg_catalog.pg_rewrite'::regclass AND dep.objid=r.oid
   LEFT JOIN pg_catalog.pg_proc p ON dep.refclassid='pg_catalog.pg_proc'::regclass AND p.oid=dep.refobjid
   LEFT JOIN pg_catalog.pg_class rc ON dep.refclassid='pg_catalog.pg_class'::regclass AND rc.oid=dep.refobjid
   JOIN pg_catalog.pg_namespace dn ON dn.oid=coalesce(p.pronamespace,rc.relnamespace)
   WHERE r.ev_class=c.oid AND dn.nspname NOT IN ('pg_catalog',$1))
FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relkind IN ('v','m') ORDER BY c.relname LIMIT 10001
"#,
    ),
    (
        "sequence",
        r#"
SELECT c.relname::text,jsonb_build_object('type',format_type(s.seqtypid,NULL),'start',s.seqstart,'increment',s.seqincrement,'min',s.seqmin,'max',s.seqmax,'cache',s.seqcache,'cycle',s.seqcycle,
 'owned_by',(SELECT tn.nspname||'.'||tc.relname||'.'||a.attname FROM pg_catalog.pg_depend d JOIN pg_catalog.pg_class tc ON tc.oid=d.refobjid JOIN pg_catalog.pg_namespace tn ON tn.oid=tc.relnamespace JOIN pg_catalog.pg_attribute a ON a.attrelid=tc.oid AND a.attnum=d.refobjsubid WHERE d.classid='pg_catalog.pg_class'::regclass AND d.objid=c.oid AND d.deptype IN ('a','i') LIMIT 1)),true
FROM pg_catalog.pg_sequence s JOIN pg_catalog.pg_class c ON c.oid=s.seqrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 ORDER BY c.relname LIMIT 10001
"#,
    ),
    (
        // Extension members are owned/granted by whoever installed the
        // extension (often the bootstrap superuser); its version covers them.
        "routine",
        r#"
SELECT p.proname||'('||pg_get_function_identity_arguments(p.oid)||')',
 jsonb_build_object('kind',p.prokind,'owner',CASE WHEN ext.oid IS NULL THEN pg_get_userbyid(p.proowner) END,'language',l.lanname,'volatility',p.provolatile,'security_definer',p.prosecdef,
 'extension',ext.extname,'definition_digest',encode(sha256(convert_to(CASE WHEN p.prokind IN ('f','p') THEN pg_get_functiondef(p.oid) ELSE p.prosrc END||coalesce(p.probin,''),'UTF8')),'hex'),
 'grants',CASE WHEN ext.oid IS NULL THEN (SELECT jsonb_agg(jsonb_build_object('grantor',pg_get_userbyid(a.grantor),'grantee',CASE WHEN a.grantee=0 THEN jsonb_build_object('public',true) ELSE to_jsonb(pg_get_userbyid(a.grantee)) END,'privilege',a.privilege_type,'grantable',a.is_grantable) ORDER BY a.grantor,a.grantee,a.privilege_type)
 FROM aclexplode(coalesce(p.proacl,acldefault('f',p.proowner))) a) END),
 p.prokind IN ('f','p') AND (l.lanname IN ('sql','plpgsql') OR (ext.extname IS NOT NULL AND en.nspname=$1))
FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace JOIN pg_catalog.pg_language l ON l.oid=p.prolang
LEFT JOIN pg_catalog.pg_depend xd ON xd.classid='pg_catalog.pg_proc'::regclass AND xd.objid=p.oid AND xd.deptype='e'
LEFT JOIN pg_catalog.pg_extension ext ON ext.oid=xd.refobjid LEFT JOIN pg_catalog.pg_namespace en ON en.oid=ext.extnamespace WHERE n.nspname=$1 ORDER BY p.proname LIMIT 10001
"#,
    ),
    (
        "trigger",
        r#"
SELECT quote_ident(c.relname)||'.'||quote_ident(t.tgname),jsonb_build_object('definition',pg_get_triggerdef(t.oid,false),'enabled',t.tgenabled),
 pn.nspname IN ('pg_catalog',$1)
FROM pg_catalog.pg_trigger t JOIN pg_catalog.pg_class c ON c.oid=t.tgrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
JOIN pg_catalog.pg_proc tp ON tp.oid=t.tgfoid JOIN pg_catalog.pg_namespace pn ON pn.oid=tp.pronamespace WHERE n.nspname=$1 AND NOT t.tgisinternal ORDER BY c.relname,t.tgname LIMIT 10001
"#,
    ),
    (
        "policy",
        r#"
SELECT quote_ident(c.relname)||'.'||quote_ident(p.polname),jsonb_build_object('command',p.polcmd,'permissive',p.polpermissive,'using',pg_get_expr(p.polqual,p.polrelid),'check',pg_get_expr(p.polwithcheck,p.polrelid),
 'roles',(SELECT jsonb_agg(CASE WHEN x=0 THEN 'PUBLIC' ELSE pg_get_userbyid(x) END ORDER BY x) FROM unnest(p.polroles) x)),
 NOT EXISTS(SELECT 1 FROM pg_catalog.pg_depend dep JOIN pg_catalog.pg_proc p ON p.oid=dep.refobjid JOIN pg_catalog.pg_namespace pn ON pn.oid=p.pronamespace WHERE dep.classid='pg_catalog.pg_policy'::regclass AND dep.objid=p.oid AND dep.refclassid='pg_catalog.pg_proc'::regclass AND pn.nspname NOT IN ('pg_catalog',$1))
FROM pg_catalog.pg_policy p JOIN pg_catalog.pg_class c ON c.oid=p.polrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 ORDER BY c.relname,p.polname LIMIT 10001
"#,
    ),
    (
        "type",
        r#"
SELECT t.typname::text,jsonb_build_object('kind',t.typtype,'owner',pg_get_userbyid(t.typowner),'definition',format_type(t.oid,NULL)),false
FROM pg_catalog.pg_type t JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace
WHERE n.nspname=$1 AND t.typrelid=0 AND t.typelem=0 ORDER BY t.typname LIMIT 10001
"#,
    ),
    (
        "extension",
        r#"
SELECT e.extname::text,jsonb_build_object('version',e.extversion,'schema',n.nspname,'external',n.nspname<>$1),n.nspname=$1
FROM pg_catalog.pg_extension e JOIN pg_catalog.pg_namespace n ON n.oid=e.extnamespace WHERE e.extname<>'plpgsql' AND $1::text IS NOT NULL ORDER BY e.extname LIMIT 10001
"#,
    ),
    (
        "other",
        r#"
SELECT 'operator.'||o.oprname,jsonb_build_object('kind','operator'),false FROM pg_catalog.pg_operator o JOIN pg_catalog.pg_namespace n ON n.oid=o.oprnamespace WHERE n.nspname=$1
UNION ALL SELECT 'collation.'||c.collname,jsonb_build_object('kind','collation','version',c.collversion),false FROM pg_catalog.pg_collation c JOIN pg_catalog.pg_namespace n ON n.oid=c.collnamespace WHERE n.nspname=$1
UNION ALL SELECT 'conversion.'||c.conname,jsonb_build_object('kind','conversion'),false FROM pg_catalog.pg_conversion c JOIN pg_catalog.pg_namespace n ON n.oid=c.connamespace WHERE n.nspname=$1
UNION ALL SELECT 'default_acl.'||pg_get_userbyid(d.defaclrole)||'.'||d.defaclobjtype::text,jsonb_build_object('kind','default_acl','external',d.defaclnamespace=0),false FROM pg_catalog.pg_default_acl d LEFT JOIN pg_catalog.pg_namespace n ON n.oid=d.defaclnamespace WHERE n.nspname=$1 OR d.defaclnamespace=0
UNION ALL SELECT 'rule.'||quote_ident(c.relname)||'.'||quote_ident(r.rulename),jsonb_build_object('kind','rule','definition',pg_get_ruledef(r.oid,false)),false FROM pg_catalog.pg_rewrite r JOIN pg_catalog.pg_class c ON c.oid=r.ev_class JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND r.rulename<>'_RETURN'
UNION ALL SELECT 'operator_class.'||quote_ident(o.opcname),jsonb_build_object('kind','operator_class'),false FROM pg_catalog.pg_opclass o JOIN pg_catalog.pg_namespace n ON n.oid=o.opcnamespace WHERE n.nspname=$1
UNION ALL SELECT 'operator_family.'||quote_ident(o.opfname),jsonb_build_object('kind','operator_family'),false FROM pg_catalog.pg_opfamily o JOIN pg_catalog.pg_namespace n ON n.oid=o.opfnamespace WHERE n.nspname=$1
UNION ALL SELECT 'text_search_parser.'||quote_ident(p.prsname),jsonb_build_object('kind','text_search'),false FROM pg_catalog.pg_ts_parser p JOIN pg_catalog.pg_namespace n ON n.oid=p.prsnamespace WHERE n.nspname=$1
UNION ALL SELECT 'text_search_template.'||quote_ident(t.tmplname),jsonb_build_object('kind','text_search'),false FROM pg_catalog.pg_ts_template t JOIN pg_catalog.pg_namespace n ON n.oid=t.tmplnamespace WHERE n.nspname=$1
UNION ALL SELECT 'security_label.'||pg_describe_object(s.classoid,s.objoid,s.objsubid)||'.'||s.provider,jsonb_build_object('kind','security_label','definition',s.label,'external',true),false FROM pg_catalog.pg_seclabel s WHERE $1::text IS NOT NULL
UNION ALL SELECT 'shared_security_label.'||pg_describe_object(s.classoid,s.objoid,0)||'.'||s.provider,jsonb_build_object('kind','security_label','definition',s.label,'external',true),false FROM pg_catalog.pg_shseclabel s WHERE $1::text IS NOT NULL
UNION ALL SELECT 'event_trigger.'||quote_ident(e.evtname),jsonb_build_object('kind','event_trigger','enabled',e.evtenabled,'external',true),false FROM pg_catalog.pg_event_trigger e WHERE $1::text IS NOT NULL
UNION ALL SELECT 'statistics.'||s.stxname,jsonb_build_object('kind','statistics'),false FROM pg_catalog.pg_statistic_ext s JOIN pg_catalog.pg_namespace n ON n.oid=s.stxnamespace WHERE n.nspname=$1
UNION ALL SELECT 'text_search_config.'||c.cfgname,jsonb_build_object('kind','text_search'),false FROM pg_catalog.pg_ts_config c JOIN pg_catalog.pg_namespace n ON n.oid=c.cfgnamespace WHERE n.nspname=$1
UNION ALL SELECT 'text_search_dictionary.'||d.dictname,jsonb_build_object('kind','text_search'),false FROM pg_catalog.pg_ts_dict d JOIN pg_catalog.pg_namespace n ON n.oid=d.dictnamespace WHERE n.nspname=$1
"#,
    ),
];
