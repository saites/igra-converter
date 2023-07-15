<script setup lang="ts">
import { 
  friendlyProblem, friendlyFix, 
//   fullName, dbContestantCat 
} from '@/utils.js'
import { ref, computed } from 'vue'
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
const isDbMismatch = computed(() => {
  let { issues, field, fields } = props;
  if (!issues) { return false; }
  let compareTo = field ? [field] : (fields ?? []);

  return issues.some((issue) => {
      return (
        issue.problem.name === "DbMismatch"
        && compareTo.some((f) => issue.problem.data.field === f)
      )
  })
})

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

</script>

<template>
  <article>
    <section>
      <header class="text-lg">Registration Data</header>

      <table class="table-auto">
        <thead>
          <tr>
            <th></th>
            <th>IGRA Number</th>
            <th>Association</th>
            <th>Legal Name</th>
            <th>Performance Name</th>
            <th>Date of Birth</th>
            <th>Competes With</th>
            <th>Address</th>
            <th>Email</th>
            <th>Cell Phone</th>
            <th>Home Phone</th>
            <th>Note to Director</th>
          </tr>
        </thead>
        <tbody>

          <tr>
            <th>Registration Data</th>
            <data-cell :value=contestant.association?.igra></data-cell>
            <data-cell :value=contestant.association?.memberAssn></data-cell>
            <data-cell :value="fullName(contestant.firstName, contestant.lastName)"></data-cell>
            <data-cell :value=contestant.performanceName></data-cell>
            <data-cell :value=contestantBday></data-cell>
            <data-cell :value=contestant.gender></data-cell>

            <address-cell
              :line1=contestant.address.addressLine1
              :line2=contestant.address.addressLine2
              :city=contestant.address.city
              :region=contestant.address.region
              :country=contestant.address.country
              :postalCode=contestant.address.zipCode
            ></address-cell>

            <data-cell :value=contestant.address.email></data-cell>
            <data-cell :value=contestant.address.cellPhoneNo></data-cell>
            <data-cell :value=contestant.address.homePhoneNo></data-cell>
            <data-cell :value=contestant.noteToDirector></data-cell>
          </tr>

          <db-record-row header="Database Record"
            :class="{ 'bg-yellow-200': !found }"
            :record=found></db-record-row>
          <db-record-row v-for="match in maybeMatches" header="Possible Match"
            class="bg-blue-200"
            :record=match></db-record-row>

        </tbody>
      </table>
    </section>

    <section>
      <header class="text-lg">Registered Events</header>
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
  </article>
</template>

<style scoped>
table {
  border: 2px solid #42b983;
  border-radius: 3px;
  background-color: #fff;
}

th {
  background-color: #42b983;
  color: rgba(255, 255, 255, 0.66);
}

tr {
  background-color: #f9f9f9;
}

article {
  @apply grid grid-cols-1 justify-items-center ;
}
</style>

