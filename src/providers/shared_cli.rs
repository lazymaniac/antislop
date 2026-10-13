//! Bounded, supervised print-mode transport shared by CLI adapters.
use super::*;
use crate::{Error, process::SupervisedChild, require};
use std::{io::{BufReader, Read, Write}, process::{Command, Stdio}, sync::{atomic::Ordering, mpsc}, time::Duration};

pub enum StreamEvent {
    Session(String),
    Text { content:String, delta:bool },
    Result { output:Option<String>, usage:UsageReport },
    Failed(FailureKind),
    ForbiddenTool,
    Unknown,
}

pub fn exact_session(provider:&str, resume:Option<&str>) -> Result<String> {
    if let Some(session) = resume {
        if uuid::Uuid::parse_str(session).is_err() {
            return Err(failure(provider,FailureKind::InvalidSession,"resume requires an exact session UUID; aliases and indices are forbidden"));
        }
        Ok(session.into())
    } else { Ok(crate::id()) }
}

pub fn error_kind(value:&Value) -> FailureKind {
    let code = value.get("code").or_else(|| value.get("status")).and_then(Value::as_i64);
    let kind = value.get("type").or_else(|| value.get("code")).and_then(Value::as_str).unwrap_or("").to_ascii_lowercase();
    if matches!(code,Some(401|403|41)) || kind.contains("auth") { FailureKind::Authentication }
    else if code == Some(429) || kind.contains("rate") || kind.contains("quota") { FailureKind::RateLimit }
    else if matches!(code,Some(500..=599)) || kind.contains("network") || kind.contains("connection") { FailureKind::Network }
    else if kind.contains("session") { FailureKind::InvalidSession }
    else { FailureKind::Process }
}

/// Capture only bounded public probe output; stderr is never retained.
pub fn capture(command:&mut Command, cancelled:Option<&AtomicBool>) -> Result<String> {
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = SupervisedChild::spawn(command)?;
    let stdout = child.child.stdout.take().expect("probe stdout");
    let (sender,receiver)=mpsc::sync_channel(1);
    let reader=std::thread::spawn(move || {
        let mut bytes=Vec::new();
        let result=stdout.take((crate::transport::MAX_FRAME+1) as u64).read_to_end(&mut bytes).map(|_|bytes);
        let _=sender.send(result);
    });
    let mut output=None;
    let mut status=None;
    while output.is_none() || status.is_none() {
        if cancelled.is_some_and(|flag|flag.load(Ordering::SeqCst)) {
            child.cancel()?;
            return Err(failure("cli",FailureKind::Cancelled,"probe stopped by cancellation"));
        }
        status=status.or(child.child.try_wait()?);
        if output.is_none() {
            match receiver.recv_timeout(Duration::from_millis(25)) {
                Ok(result)=> { let bytes=result?; require(bytes.len()<=crate::transport::MAX_FRAME,"CLI probe exceeds transport limit")?; output=Some(bytes); }
                Err(mpsc::RecvTimeoutError::Timeout)=>(),
                Err(mpsc::RecvTimeoutError::Disconnected)=>return Err(failure("cli",FailureKind::Protocol,"probe reader failed")),
            }
        } else if status.is_none() { std::thread::sleep(Duration::from_millis(25)); }
    }
    let _=reader.join();
    require(status.expect("probe exit").success(),"CLI probe failed")?;
    String::from_utf8(output.expect("probe output")).map_err(|_|failure("cli",FailureKind::Protocol,"probe output is not UTF-8"))
}

pub fn inspect(provider:&str,binary:&str,version:&str,flags:&[&str],cancelled:Option<&AtomicBool>) -> Result<String> {
    let output=capture(crate::process::command(binary)?.arg("--version"),cancelled)?;
    if !output.trim().starts_with(version) { return Err(failure(provider,FailureKind::Unsupported,"CLI version is outside the validated contract")); }
    let help=capture(crate::process::command(binary)?.arg("--help"),cancelled)?;
    if !flags.iter().all(|flag|help.contains(flag)) { return Err(failure(provider,FailureKind::Unsupported,"CLI does not advertise the required restriction/transport flags")); }
    Ok(output.trim().into())
}

pub fn run(
    provider:&str, command:&mut Command, request:&TurnRequest<'_>, session_id:&str,
    prompt:String, cancelled:Arc<AtomicBool>, events:&mut dyn FnMut(&ProviderEvent)->Result<()>,
    normalize:fn(&Value)->Result<StreamEvent>,
) -> Result<TurnReceipt> {
    require(!request.turn_id.trim().is_empty(),"missing durable turn ID")?;
    if cancelled.load(Ordering::SeqCst) { return Err(failure(provider,FailureKind::Cancelled,"turn cancelled before start")); }
    // Callers persist these callbacks before permitting the following effect.
    events(&ProviderEvent::DeliveryPending {turn_id:request.turn_id.into(),session_id:session_id.into()})?;
    command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child=SupervisedChild::spawn(command)?;
    let stdout=child.child.stdout.take().expect("provider stdout");
    let stderr=child.child.stderr.take().expect("provider stderr");
    let diagnostics=std::thread::spawn(move||{let _=std::io::copy(&mut BufReader::new(stderr),&mut std::io::sink());});
    let (sender,receiver)=mpsc::sync_channel(16);
    let reader=std::thread::spawn(move||{
        let mut reader=BufReader::new(stdout);
        loop {
            let frame=crate::transport::read::<Value>(&mut reader);
            let terminal=matches!(frame,Ok(None)|Err(_));
            if sender.send(frame).is_err() || terminal {break;}
        }
    });
    let mut stdin=child.child.stdin.take().expect("provider stdin");
    let (sent_sender,sent_receiver)=mpsc::sync_channel(1);
    let writer=std::thread::spawn(move||{
        let result=stdin.write_all(prompt.as_bytes()).and_then(|_|stdin.flush());
        drop(stdin);
        let _=sent_sender.send(result);
    });
    let mut delivered=false;
    let mut observed=false;
    let mut completed=false;
    let mut eof=false;
    let mut status=None;
    let mut output=String::new();
    let mut usage=UsageReport::Unavailable;
    let mut failed=None;
    while !eof || status.is_none() {
        if cancelled.load(Ordering::SeqCst) {
            let callback=events(&ProviderEvent::CancellationRequested{turn_id:request.turn_id.into()});
            child.cancel()?;
            callback?;
            events(&ProviderEvent::Stopped{turn_id:request.turn_id.into()})?;
            return Err(ProviderFailure {provider:provider.into(),kind:FailureKind::Cancelled,message:"process group stopped; prior effects require reconciliation".into(),session_id:Some(session_id.into()),stopped:true,partial_effects:delivered}.into());
        }
        if !delivered && let Ok(result)=sent_receiver.try_recv() {
            result?;
            delivered=true;
            events(&ProviderEvent::DeliverySent{turn_id:request.turn_id.into()})?;
        }
        status=status.or(child.child.try_wait()?);
        if eof {std::thread::sleep(Duration::from_millis(25));continue;}
        let frame=match receiver.recv_timeout(Duration::from_millis(25)) {
            Ok(Ok(Some(value)))=>value,
            Ok(Ok(None))=>{eof=true;continue;},
            Ok(Err(_))=>return Err(failure(provider,FailureKind::Protocol,"malformed, duplicate-key or oversized CLI frame")),
            Err(mpsc::RecvTimeoutError::Timeout)=>continue,
            Err(mpsc::RecvTimeoutError::Disconnected)=>return Err(failure(provider,FailureKind::Protocol,"CLI event stream disconnected")),
        };
        match normalize(&frame)? {
            StreamEvent::Session(id)=> {
                if id!=session_id {return Err(failure(provider,FailureKind::InvalidSession,"CLI returned a different session than the durable requested ID"));}
                if !observed {events(&ProviderEvent::SessionObserved{session_id:id})?;observed=true;}
            }
            StreamEvent::Text{content,delta}=> {
                if completed {return Err(failure(provider,FailureKind::Protocol,"text after terminal result"));}
                if delta {output.push_str(&content);} else {output=content;}
                require(output.len()<=crate::transport::MAX_FRAME,"provider artifact exceeds transport limit")?;
            }
            StreamEvent::Result{output:result,usage:reported}=> {
                if completed {return Err(failure(provider,FailureKind::Protocol,"multiple terminal results"));}
                completed=true;
                if let Some(value)=result {output=value;}
                usage=reported;
            }
            StreamEvent::Failed(kind)=>failed=Some(kind),
            StreamEvent::ForbiddenTool=>return Err(failure(provider,FailureKind::Policy,"tool/subagent event violates the tool-free profile")),
            StreamEvent::Unknown=>(),
        }
    }
    let _=writer.join();
    if !delivered {
        sent_receiver.recv().map_err(|_|failure(provider,FailureKind::Protocol,"prompt writer failed"))??;
        delivered=true;
        events(&ProviderEvent::DeliverySent{turn_id:request.turn_id.into()})?;
    }
    let _=reader.join();let _=diagnostics.join();
    let status=status.expect("provider exit");
    if failed.is_some() || !status.success() {
        let kind=if status.code()==Some(41) {FailureKind::Authentication} else {failed.unwrap_or(FailureKind::Process)};
        return Err(ProviderFailure {provider:provider.into(),kind,message:"CLI turn failed; raw diagnostics intentionally not retained".into(),session_id:Some(session_id.into()),stopped:true,partial_effects:delivered}.into());
    }
    if !completed || !observed {return Err(failure(provider,FailureKind::Protocol,"CLI exited without a session and successful terminal result"));}
    let _:Value=crate::transport::decode(output.as_bytes()).map_err(|_|failure(provider,FailureKind::Protocol,"CLI artifact is not strict JSON"))?;
    events(&ProviderEvent::Completed{turn_id:request.turn_id.into()})?;
    Ok(TurnReceipt {turn_id:request.turn_id.into(),provider:provider.into(),session_id:session_id.into(),delivered,completed:true,stopped:true,output,usage})
}
