"""Command-owned output projection and independent terminal verdict dimensions."""

def command_frames(steps, lines, phase='steps'):
    if not isinstance(steps, list) or len(steps) != len(lines):
        raise ValueError('command provenance: missing or extra steps')
    frames=[]
    for index,(step,line) in enumerate(zip(steps,lines)):
        if not isinstance(step,dict) or type(step.get('index')) is not int or step['index']!=index:
            raise ValueError('command provenance: missing, duplicate or reordered step index')
        outputs=step.get('outputs')
        if not isinstance(line,str) or not isinstance(outputs,list):
            raise ValueError('command provenance: malformed command or outputs')
        if line.strip().split(maxsplit=1)[:1]==['extract'] and sum(isinstance(frame,dict) and frame.get('kind')=='structured' for frame in outputs)!=1:
            raise ValueError('extraction command requires exactly one structured result')
        for ordinal,frame in enumerate(outputs):
            if not isinstance(frame,dict):
                raise ValueError('command provenance: malformed output')
            frames.append(dict(frame,command_provenance=dict(phase=phase,step=index,oil=line,output=ordinal)))
    return frames


def extraction_values(frames, lines):
    """Keep extraction values regardless of their JSON type; reject ambiguous custody."""
    values=[];seen=set();last={};extractions={}
    for frame in frames:
        if frame.get('kind')!='structured':continue
        owner=frame.get('command_provenance')
        if not isinstance(owner,dict):raise ValueError('structured output lacks command provenance')
        phase,index,ordinal=owner.get('phase'),owner.get('step'),owner.get('output')
        if phase not in ('steps','recovery') or type(index) is not int or index<0 or type(ordinal) is not int or ordinal<0:
            raise ValueError('malformed output command provenance')
        command=owner.get('oil')
        if not isinstance(command,str) or (phase=='steps' and (index>=len(lines) or command!=lines[index])):
            raise ValueError('output command differs from source task')
        key=(phase,index,ordinal)
        if key in seen or (phase in last and (index,ordinal)<=last[phase]):
            raise ValueError('duplicate or reordered command output')
        seen.add(key);last[phase]=(index,ordinal)
        if command.strip().split(maxsplit=1)[:1]==['extract']:
            extractions[(phase,index)]=extractions.get((phase,index),0)+1
            values.append(frame.get('value'))
    expected_steps={index for index,line in enumerate(lines) if line.strip().split(maxsplit=1)[:1]==['extract']}
    if any(extractions.get(('steps',index))!=1 for index in expected_steps) or any(count!=1 for count in extractions.values()):
        raise ValueError('extraction command result cardinality mismatch')
    return values


def terminal_verdict(functional_errors, accountings, cleanups):
    resource_errors=[];cleanup_errors=[]
    if not accountings:resource_errors.append('missing resource accounting')
    for index,row in enumerate(accountings):
        if not isinstance(row,dict) or row.get('complete') is not True or row.get('errors') != []:
            resource_errors.append(dict(wave=index,error='resource accounting incomplete',details=row))
    if not cleanups:cleanup_errors.append('missing cleanup evidence')
    for index,row in enumerate(cleanups):
        if not isinstance(row,dict) or row.get('group_settled') is not True or row.get('errors') != [] or row.get('cleanup_uncertain'):
            cleanup_errors.append(dict(wave=index,error='cleanup not settled',details=row))
    functional=list(functional_errors)
    return dict(functional_passed=not functional,resource_complete=not resource_errors,
                cleanup_complete=not cleanup_errors,operation_errors=functional,
                resource_errors=resource_errors,cleanup_errors=cleanup_errors,
                passed=not functional and not resource_errors and not cleanup_errors,
                errors=functional+['resource accounting incomplete']*bool(resource_errors)+['cleanup not settled']*bool(cleanup_errors))
