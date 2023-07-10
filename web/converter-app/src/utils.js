
const PROBLEM_TO_MESSAGE = {
// General validation issues.
"NoValue": "The registrant didn't fill this field.",
"NotOldEnough": "The DoB indicates the registrant is too young to participate.",
"NotEnoughRounds": "The registrant is not registered for enough go-rounds.",

// Database record matching issues.
"NotAMember": "The registrant says they are not a member.",
"MaybeAMember": "The registrant says they are not a member, but there is a database record that closely matches their information.",
"NoPerfectMatch": "We couldn't find a database record that matches the registrant's information",
"DbMismatch": "The registrant entered values different from the current database value.",

// Partner issues.
"UnknownPartner": "We can't associate the entered partner info with a single database record.",
"TooFewPartners": "The registrant didn't list enough partners.",
"UnregisteredPartner": "This partner isn't registered for this rodeo.",
"MismatchedPartners": "This partner is registered for this rodeo, but didn't list the registrant as a partner for this event and round.",

// Likely programming errors.
"UnknownEventID": "The (input) registration record's event ID does not match a known value.",
"InvalidRoundID": "The (input) registration record's round ID is invalid.",
"TooManyPartners": "The (input) registration record lists more partners for this event that are allowed."
};

export function friendlyProblem(issue) {
  return PROBLEM_TO_MESSAGE[issue.problem.name] ?? "Unexpected issue."
}

const FIX_TO_MESSAGE = {
    "UpdateDatabase": "Update the database.",
    "AddNewMember": "Add this member.",
    "ContactRegistrant": "Contact the registrant.",
    "ContactDevelopers": "Contact the developers :(",
    "UseThisRecord": "Use this record.",
}

export function friendlyFix(issue) {
  return (
    issue.fix.name === "UseThisRecord"
    ? `They might have meant ${issue.fix.data}.`
    : FIX_TO_MESSAGE[issue.fix.name]
  )
}

export function fullName(first, last) {
  if (!first) { return last ?? undefined; }
  if (!last) { return first ?? undefined; }
  return `${first} ${last}`
}

export function dbContestantCat(value) {
  if (!value) { return }
  return value === "M" ? "Cowboys" : ("F" ? "Cowgirls" : "Unknown")
}

