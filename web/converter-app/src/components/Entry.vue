<script setup lang="ts">
import { friendlyProblem, friendlyFix } from '@/utils.js'
import { ref, computed } from 'vue'
import Accordion from "./Accordion.vue"
import EventsGrid from "./Events.vue"
import EventRow from "./EventRow.vue"
import DataCell from "./DataCell.vue"
import AddressCell from "./AddressCell.vue"
import DbRecordRow from "./DbRecordRow.vue"

const props = defineProps<{
    contestant: Object,
    partners: Array,
    found: Object,
    events: Array,
    issues: Array,
    relevant: Object,
}>()

function fullName(first, last) {
  if (!first) { return last ?? undefined; }
  if (!last) { return first ?? undefined; }
  return `${first} ${last}`
}

function dbContestantCat(value) {
  if (!value) { return }
  return value === "M" ? "Cowboys" : ("F" ? "Cowgirls" : "Unknown")
}

const contestantBday = computed(() => {
  let { contestant } = props;
  return `${contestant.dob.month}/${contestant.dob.day}/${contestant.dob.year}`
})

// return true if the given issue's problem matches any of the given fields
function isThisField(issue, fields) {
  return compareTo.some((f) => issue.problem.data.field === f)
}

// fields the registrant didn't fill in
const unfilledFields = computed(() => {
  let { issues } = props;
  if (!issues) { return false; }

  return (issues
    .filter((issue) => issue.problem.name === "NoValue")
    .map((issue) => issue.problem.data.field)
  )
})

// Do we have a database value, but different from this one?
function isDbMismatch(fields) {
  let { issues } = props;
  if (!issues) { return false; }

  return false

  return issues.some((issue) => {
      return (
        issue.problem.name === "DbMismatch"
        && fields.some((f) => issue.problem.data.field === f)
      )
  })
}

// Should we expect _any_ DB data?
const missingDbValue = computed(() => {
  let { issues } = props;
  if (!issues) { return false; }
  return issues.some((issue) => {
    return (
      issue.problem.name === "NotAMember"
      || issue.problem.name === "MaybeAMember"
      || issue.problem.name === "NoPerfectMatch"
    )
  });
})

// Possible matches to the registrant
const maybeMatches = computed(() => {
  let { issues, relevant } = props
  if (!issues || !relevant) { return [] }

  return (
    issues
      .filter((issue) => issue.problem.name === "NoPerfectMatch" && issue.fix.name === "UseThisRecord")
      .map((issue) => relevant[issue.fix.data])
      .filter((record) => record !== undefined)
  )
})

const contestantRegion = computed(() => {
  // The region in the registration form is a full state name,
  // which looks pretty awkward.
  // So, if we have a matching DB value, use that instead,
  // since those are just two letter abbreviations.
  let { issues, contestant, relevant, found } = props
  if (!issues || !contestant || !relevant) { return }
  
  // Make sure we have a DB value to use.
  if (isDbMismatch(["Region"])) { 
    return contestant.address.region
  }

  return found?.state ?? contestant.address.region
})

</script>

<template>
  <article class="grid grid-cols-1 justify-items-center gap-y-2">
    <accordion class="grid grid-cols-12 w-[800px] min-w-full max-w-screen-xlg">
      <template #summary>
        <div :class="{'bg-gray-200': issues.length === 0, 'bg-red-200': issues.length > 0}">
        <header class="text-lg">
          <span>{{fullName(contestant.firstName, contestant.lastName)}}</span>
          <span class="mx-8">{{events.length}} Go-Rounds</span>
          <span>({{issues.length}} 
            issue{{issues.length !== 1 ? 's' : ''}}
            with this registration)</span>
        </header>
        </div>
      </template>

      <template #content>
        <section>
          <header class="text-md">Registration Info</header>
          <div class="grid grid-cols-2">
            <span><em>IGRA Number</em></span>
            <data-cell :value=contestant.association?.igra></data-cell>

            <span><em>Association</em></span>
            <data-cell :value=contestant.association?.memberAssn></data-cell>

            <span><em>Legal Name</em></span>
            <data-cell :value="fullName(contestant.firstName, contestant.lastName)"></data-cell>
            <span><em>Performance Name</em></span>
            <data-cell :value=contestant.performanceName></data-cell>

            <span><em>Date of Birth</em></span>
            <data-cell :value=contestantBday></data-cell>

            <span><em>Competes With</em></span>
            <data-cell :value=contestant.gender></data-cell>

            <span><em>Address</em></span>
            <address-cell
              :line1=contestant.address.addressLine1
              :line2=contestant.address.addressLine2
              :city=contestant.address.city
              :region="contestantRegion"
              :country=contestant.address.country
              :postalCode=contestant.address.zipCode
            ></address-cell>

            <span><em>Email</em></span>
            <data-cell :value=contestant.address.email></data-cell>

            <span><em>Cell Phone</em></span>
            <data-cell :value=contestant.address.cellPhoneNo></data-cell>

            <span><em>Home Phone</em></span>
            <data-cell :value=contestant.address.homePhoneNo></data-cell>

            <span><em>Note to Director</em></span>
            <data-cell :value=contestant.noteToDirector></data-cell>
          </div>

          <db-record-row header="Database Record"
            :class="{ 'bg-yellow-200': !found }"
            :record=found></db-record-row>
          <db-record-row v-for="match in maybeMatches" header="Possible Match"
            class="bg-blue-200"
            :record=match></db-record-row>
        </section>
    
        <section>
          <events-grid 
            :events=events
            :issues=issues
            :relevant=relevant
            :partners=partners
          ></events-grid>
        </section>

          <section v-if="issues.length > 0">
            <header class="text-lg">Problems</header>
            <table class="table-auto">
              <thead>
                <tr>
                  <th>Problem</th>
                  <th></th>
                  <th>Suggestion</th>
                  <th></th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="issue in issues">
                  <td>{{issue.problem.name}}</td>
                  <td><template v-if="issue.problem.data">{{issue.problem.data}}</template></td>
                  <td>{{issue.fix.name}}</td>
                  <td><template v-if="issue.fix.data">{{issue.fix.data}}</template></td>
                </tr>
              </tbody>
            </table>
          </section>
      </template>
    </accordion>
  </article>
</template>

<style>
span:nth-child(odd) {
  @apply text-end pe-4;
}
</style>

